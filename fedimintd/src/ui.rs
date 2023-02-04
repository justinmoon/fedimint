use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use aead::{encrypted_read, get_key};
use anyhow::{format_err, Error};
use askama::Template;
use axum::extract::Form;
use axum::response::{IntoResponse, Redirect, Response};
use axum::{
    routing::{get, post},
    Router,
};
use axum_macros::debug_handler;
use bitcoin::Network;
use fedimint_api::bitcoin_rpc::BitcoindRpcBackend;
use fedimint_api::config::{ClientConfig, ModuleGenRegistry};
use fedimint_api::task::TaskGroup;
use fedimint_api::Amount;
use fedimint_core::api::WsClientConnectInfo;
use fedimint_core::util::SanitizedUrl;
use fedimint_server::config::io::{
    create_cert, encrypted_json_write, parse_peer_params, run_dkg, write_nonprivate_configs,
    CONSENSUS_CONFIG, JSON_EXT, PRIVATE_CONFIG, SALT_FILE, TLS_PK,
};
use http::StatusCode;
use qrcode_generator::QrCodeEcc;
use serde::Deserialize;
use tokio::select;
use tokio::sync::mpsc::Sender;
use tokio::sync::Mutex;
use tokio_rustls::rustls;
use tracing::{debug, error};
use url::Url;

use crate::{configure_modules, module_registry, CODE_VERSION};

#[derive(Deserialize, Debug, Clone)]
#[allow(dead_code)]
pub struct Guardian {
    name: String,
    tls_connect_string: String,
}

#[derive(Template)]
#[template(path = "home.html")]
struct HomeTemplate {}

async fn home_page(axum::extract::State(_): axum::extract::State<MutableState>) -> HomeTemplate {
    HomeTemplate {}
}

#[derive(Template)]
#[template(path = "run.html")]
struct RunTemplate {
    state: RunTemplateState,
}

enum RunTemplateState {
    DkgNotStarted,
    DkgInProgress,
    DkgDone(String),   // connnection string
    DkgFailed(String), // error
    LocalIoError(String),
}

async fn run_page(axum::extract::State(state): axum::extract::State<MutableState>) -> RunTemplate {
    let state = state.lock().await;

    RunTemplate {
        state: match state.dkg_state {
            Some(DkgState::Success) => {
                let path = state.data_dir.join("client.json");
                // TODO: refactor be a standalone function
                match std::fs::File::open(path) {
                    Ok(file) => match serde_json::from_reader(file) {
                        Ok(cfg) => {
                            let connect_info = WsClientConnectInfo::from_honest_peers(&cfg);

                            RunTemplateState::DkgDone(
                                serde_json::to_string(&connect_info).expect("should deserialize"),
                            )
                        }
                        Err(e) => RunTemplateState::LocalIoError(e.to_string()),
                    },
                    Err(e) => RunTemplateState::LocalIoError(e.to_string()),
                }
            }
            Some(DkgState::Failure(ref e)) => RunTemplateState::DkgFailed(e.to_owned()),
            Some(DkgState::Running) => RunTemplateState::DkgInProgress,
            None => RunTemplateState::DkgNotStarted,
        },
    }
}

#[derive(Template)]
#[template(path = "add_guardians.html")]
struct AddGuardiansTemplate {
    num_guardians: u32,
    connect_string: String,
}

async fn add_guardians_page(
    axum::extract::State(state): axum::extract::State<MutableState>,
) -> AddGuardiansTemplate {
    let state = state.lock().await;
    let params = state.params.clone().expect("invalid state");
    AddGuardiansTemplate {
        num_guardians: params.num_guardians,
        connect_string: params.guardian.tls_connect_string,
    }
}

#[derive(Deserialize, Debug, Clone)]
#[allow(dead_code)]
pub struct GuardiansForm {
    connection_strings: String,
}

#[debug_handler]
async fn post_guardians(
    axum::extract::State(state): axum::extract::State<MutableState>,
    Form(form): Form<GuardiansForm>,
) -> Result<Redirect, UIError> {
    let state_copy = state.clone();
    let mut state = state.lock().await;
    let params = state.params.clone().expect("invalid state");
    let connection_strings: Vec<String> = serde_json::from_str(&form.connection_strings)
        .map_err(|_| anyhow::anyhow!("Invalid connection string"))?;
    let mut connection_strings: Vec<String> = connection_strings
        .into_iter()
        .map(|s| s.trim().to_string())
        .collect();

    connection_strings.push(params.guardian.tls_connect_string);

    // Don't allow re-running DKG if configs already exist
    let consensus_path = state
        .data_dir
        .join(CONSENSUS_CONFIG)
        .with_extension(JSON_EXT);
    if std::path::Path::new(&consensus_path).exists() {
        return Ok(Redirect::to("/run"));
    }

    // Make vec of guardians
    let params = state.params.clone().expect("invalid state");
    let mut guardians = vec![params.guardian.clone()];
    for connection_string in connection_strings.clone().into_iter() {
        guardians.push(Guardian {
            name: parse_peer_params(connection_string.clone())?.name,
            tls_connect_string: connection_string,
        });
    }

    // Actually run DKG
    let key = get_key(Some(state.password.clone()), state.data_dir.join(SALT_FILE))?;
    let pk_bytes = encrypted_read(&key, state.data_dir.join(TLS_PK))?;
    let max_denomination = Amount::from_msats(100000000000);
    let dir_out_path = state.data_dir.clone();
    let fedimintd_sender = state.sender.clone();

    // kill dkg if it's already running
    if let Some(dkg_task_group) = state.dkg_task_group.clone() {
        tracing::info!("killing dkg task group");
        dkg_task_group
            .shutdown_join_all(None)
            .await
            .expect("couldn't shut down dkg task group");
        state_copy.lock().await.dkg_state = None;
    }

    let mut dkg_task_group = state.task_group.make_subgroup().await;
    state.dkg_task_group = Some(dkg_task_group.clone());
    let module_gens = state.module_gens.clone();
    state
        .task_group
        .spawn("admin UI running DKG", move |_| async move {
            tracing::info!("Running DKG");

            state_copy.lock().await.dkg_state = Some(DkgState::Running);
            let maybe_config = run_dkg(
                params.bind_p2p,
                params.bind_api,
                &dir_out_path,
                params.federation_name,
                connection_strings,
                rustls::PrivateKey(pk_bytes),
                &mut dkg_task_group,
                CODE_VERSION,
                configure_modules(max_denomination, params.network, params.finality_delay),
                module_registry(),
            )
            .await;

            let write_result = maybe_config.and_then(|server| {
                encrypted_json_write(&server.private, &key, dir_out_path.join(PRIVATE_CONFIG))?;
                write_nonprivate_configs(&server, dir_out_path, &module_gens)
            });

            match write_result {
                Ok(_) => {
                    tracing::info!("DKG succeeded");
                    // Shut down DKG to prevent port collisions
                    dkg_task_group
                        .shutdown_join_all(None)
                        .await
                        .expect("couldn't shut down DKG task group");
                    // Tell this route that DKG succeeded
                    state_copy.lock().await.dkg_state = Some(DkgState::Success);
                    // Tell fedimint that DKG succeeded
                    fedimintd_sender
                        .send(UiMessage::DkgSuccess)
                        .await
                        .expect("failed to send over channel");
                }
                Err(e) => {
                    tracing::info!("DKG failed {:?}", e);
                    state_copy.lock().await.dkg_state = Some(DkgState::Failure(e.to_string()));
                }
            };
        })
        .await;

    Ok(Redirect::to("/run"))
}

#[derive(Template)]
#[template(path = "params.html")]
struct UrlConnection {
    ro_bitcoin_rpc_type: &'static str,
    ro_bitcoin_rpc_url: String,
}

async fn params_page(
    axum::extract::State(_state): axum::extract::State<MutableState>,
) -> UrlConnection {
    let (ro_bitcoin_rpc_type, ro_bitcoin_rpc_url) =
        match fedimint_api::bitcoin_rpc::read_bitcoin_backend_from_global_env() {
            Ok(BitcoindRpcBackend::Bitcoind(url)) => {
                let url_str = format!("{}", SanitizedUrl::new_borrowed(&url));
                ("bitcoind", url_str)
            }
            Ok(BitcoindRpcBackend::Electrum(url)) => {
                let url_str = format!("{}", SanitizedUrl::new_borrowed(&url));
                ("electrum", url_str)
            }
            Err(e) => ("error", e.to_string()),
        };
    UrlConnection {
        ro_bitcoin_rpc_type,
        ro_bitcoin_rpc_url,
    }
}

#[derive(Deserialize, Debug, Clone)]
#[allow(dead_code)]
pub struct ParamsForm {
    /// Our node name, must be unique among peers
    guardian_name: String,
    /// Federation name, same for all peers
    federation_name: String,
    /// Our API address for clients to connect to us
    api_url: Url,
    /// Our external address for communicating with our peers
    p2p_url: Url,
    /// Address we bind to for exposing the API
    bind_api: SocketAddr,
    /// Address we bind to for federation communication
    bind_p2p: SocketAddr,
    /// How many participants in federation consensus
    guardians_count: u32,
    /// Which bitcoin network the federation is using
    network: Network,
    /// The number of confirmations a deposit transaction requires before
    /// accepted by the federation
    block_confirmations: u32,
}

#[debug_handler]
async fn post_federation_params(
    axum::extract::State(state): axum::extract::State<MutableState>,
    Form(form): Form<ParamsForm>,
) -> Result<Redirect, UIError> {
    let mut state = state.lock().await;

    if !state.data_dir.exists() {
        return Err(format_err!("{:?} does not exist!", state.data_dir).into());
    }

    let tls_connect_string = create_cert(
        state.data_dir.clone(),
        form.p2p_url.clone(),
        form.api_url.clone(),
        form.guardian_name.clone(),
        Some(state.password.clone()),
    )?;

    // Update state
    state.params = Some(FederationParameters {
        federation_name: form.federation_name,
        num_guardians: form.guardians_count,
        bind_api: form.bind_api,
        bind_p2p: form.bind_p2p,
        guardian: Guardian {
            name: form.guardian_name,
            tls_connect_string,
        },
        // finality delay is always one less than required block confirmations
        finality_delay: form.block_confirmations.saturating_sub(1),
        network: form.network,
    });

    Ok(Redirect::to("/add_guardians"))
}

pub struct UIError(pub StatusCode, pub String);

impl From<anyhow::Error> for UIError {
    fn from(error: Error) -> Self {
        UIError(StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
    }
}

impl IntoResponse for UIError {
    fn into_response(self) -> Response {
        let UIError(status, msg) = self;
        (status, msg).into_response()
    }
}

async fn qr(axum::extract::State(state): axum::extract::State<MutableState>) -> impl IntoResponse {
    let state = state.lock().await;
    let path = state.data_dir.join("client.json");
    let connection_string: String = match std::fs::File::open(path) {
        Ok(file) => {
            let cfg: ClientConfig =
                serde_json::from_reader(file).expect("Could not parse cfg file.");
            let connect_info = WsClientConnectInfo::from_honest_peers(&cfg);
            serde_json::to_string(&connect_info).expect("should deserialize")
        }
        Err(_) => "".into(),
    };
    let png_bytes: Vec<u8> =
        qrcode_generator::to_png_to_vec(connection_string, QrCodeEcc::Low, 1024).unwrap();
    ([(axum::http::header::CONTENT_TYPE, "image/png")], png_bytes)
}

// FIXME: this is so similar to ParamsForm ...
#[derive(Clone)]
struct FederationParameters {
    federation_name: String,
    guardian: Guardian,
    num_guardians: u32,
    finality_delay: u32,
    network: Network,
    bind_api: SocketAddr,
    bind_p2p: SocketAddr,
}

struct State {
    params: Option<FederationParameters>,
    data_dir: PathBuf,
    sender: Sender<UiMessage>,
    password: String,
    task_group: TaskGroup,
    dkg_task_group: Option<TaskGroup>,
    module_gens: ModuleGenRegistry,
    dkg_state: Option<DkgState>,
}
type MutableState = Arc<Mutex<State>>;

#[derive(Debug)]
pub enum DkgState {
    Running,
    Success,
    Failure(String),
}

#[derive(Debug)]
pub enum UiMessage {
    DkgSuccess,
    DkgFailure(String),
}

pub async fn run_ui(
    data_dir: PathBuf,
    sender: Sender<UiMessage>,
    bind_addr: SocketAddr,
    password: String,
    task_group: TaskGroup,
    module_gens: ModuleGenRegistry,
) {
    let state = Arc::new(Mutex::new(State {
        params: None,
        data_dir,
        sender,
        password,
        task_group: task_group.clone(),
        dkg_task_group: None,
        module_gens,
        dkg_state: None,
    }));

    let app = Router::new()
        .route("/", get(home_page))
        .route("/federation_params", get(params_page))
        .route("/post_federation_params", post(post_federation_params))
        .route("/add_guardians", get(add_guardians_page))
        .route("/post_guardians", post(post_guardians))
        .route("/run", get(run_page))
        .route("/qr", get(qr))
        .with_state(state);

    let shutdown_future = task_group.make_handle().make_shutdown_rx().await;
    let server_future = axum::Server::bind(&bind_addr).serve(app.into_make_service());

    debug!("Starting setup UI server");
    select! {
        _ = shutdown_future => {
            debug!("Setup UI server shutting down");
        },
        Err(err) = server_future => {
            error!(?err, "Setup UI server encountered an error");
            panic!("Setup UI server crashed");
        }
    }
}
