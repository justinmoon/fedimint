pub mod actor;
pub mod client;
pub mod lnd;
pub mod lnrpc_client;
pub mod ng;
pub mod rpc;
pub mod types;
pub mod utils;

pub mod gatewaylnrpc {
    tonic::include_proto!("gatewaylnrpc");
}

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::anyhow;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use bitcoin::Address;
use bitcoin_hashes::hex::ToHex;
use clap::Subcommand;
use fedimint_client::module::gen::{ClientModuleGenRegistry, IClientModuleGen};
use fedimint_client::secret::PlainRootSecretStrategy;
use fedimint_client::{Client, ClientBuilder};
use fedimint_client_legacy::ln::PayInvoicePayload;
use fedimint_client_legacy::modules::ln::route_hints::RouteHint;
use fedimint_client_legacy::{ClientError, GatewayClient, GatewayClientConfig};
use fedimint_core::api::{FederationError, WsClientConnectInfo};
use fedimint_core::config::{load_from_file, ClientConfig, FederationId};
use fedimint_core::db::mem_impl::MemDatabase;
use fedimint_core::module::registry::ModuleDecoderRegistry;
use fedimint_core::task::{self, RwLock, TaskGroup, TaskHandle};
use fedimint_core::{Amount, TransactionId};
use fedimint_ln_client::contracts::Preimage;
use fedimint_mint_client::MintClientGen;
use fedimint_wallet_client::WalletClientGen;
use gatewaylnrpc::GetNodeInfoResponse;
use lightning::routing::gossip::RoutingFees;
use lnrpc_client::ILnRpcClient;
use rpc::{FederationInfo, LightningReconnectPayload};
use secp256k1::PublicKey;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};
use url::Url;

use crate::actor::GatewayActor;
use crate::client::DynGatewayClientBuilder;
use crate::lnd::GatewayLndClient;
use crate::lnrpc_client::NetworkLnRpcClient;
use crate::ng::GatewayClientGen;
use crate::rpc::rpc_server::run_webserver;
use crate::rpc::{
    BackupPayload, BalancePayload, ConnectFedPayload, DepositAddressPayload, DepositPayload,
    GatewayInfo, GatewayRequest, GatewayRpcSender, InfoPayload, RestorePayload, WithdrawPayload,
};

const ROUTE_HINT_RETRIES: usize = 10;
const ROUTE_HINT_RETRY_SLEEP: Duration = Duration::from_secs(2);
/// LND HTLC interceptor can't handle SCID of 0, so start from 1
const INITIAL_SCID: u64 = 1;

pub const DEFAULT_FEES: RoutingFees = RoutingFees {
    /// Base routing fee. Default is 0 msat
    base_msat: 0,
    /// Liquidity-based routing fee in millionths of a routed amount.
    /// In other words, 10000 is 1%. The default is 10000 (1%).
    proportional_millionths: 10000,
};

pub type Result<T> = std::result::Result<T, GatewayError>;

#[derive(Debug, Clone, Subcommand, Serialize, Deserialize)]
pub enum LightningMode {
    #[clap(name = "lnd")]
    Lnd {
        /// LND RPC address
        #[arg(long = "lnd-rpc-host", env = "FM_LND_RPC_ADDR")]
        lnd_rpc_addr: String,

        /// LND TLS cert file path
        #[arg(long = "lnd-tls-cert", env = "FM_LND_TLS_CERT")]
        lnd_tls_cert: String,

        /// LND macaroon file path
        #[arg(long = "lnd-macaroon", env = "FM_LND_MACAROON")]
        lnd_macaroon: String,
    },
    #[clap(name = "cln")]
    Cln {
        #[arg(long = "cln-extension-addr", env = "FM_GATEWAY_LIGHTNING_ADDR")]
        cln_extension_addr: Url,
    },
}

#[derive(Debug, Error)]
pub enum GatewayError {
    #[error("Federation client operation error: {0:?}")]
    ClientError(#[from] ClientError),
    #[error("Lightning rpc operation error: {0:?}")]
    LnRpcError(#[from] tonic::Status),
    #[error("Federation error: {0:?}")]
    FederationError(#[from] FederationError),
    #[error("Other: {0:?}")]
    Other(#[from] anyhow::Error),
    #[error("Failed to fetch route hints")]
    FailedToFetchRouteHints,
}

impl GatewayError {
    pub fn other(msg: String) -> Self {
        error!(msg);
        GatewayError::Other(anyhow!(msg))
    }
}

impl IntoResponse for GatewayError {
    fn into_response(self) -> Response {
        let mut err = Cow::<'static, str>::Owned(format!("{self:?}")).into_response();
        *err.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
        err
    }
}
pub struct Gateway {
    lnrpc: Arc<dyn ILnRpcClient>,
    lightning_mode: Option<LightningMode>,
    clients: Arc<RwLock<BTreeMap<FederationId, Client>>>,
    sender: mpsc::Sender<GatewayRequest>,
    receiver: mpsc::Receiver<GatewayRequest>,
    task_group: TaskGroup,
    channel_id_generator: AtomicU64,
    fees: RoutingFees,
    data_dir: PathBuf,
}

impl Gateway {
    #[allow(clippy::too_many_arguments)]
    pub async fn new(
        lightning_mode: LightningMode,
        task_group: TaskGroup,
        fees: RoutingFees,
        data_dir: PathBuf,
    ) -> Result<Self> {
        // Create message channels for the webserver
        let (sender, receiver) = mpsc::channel::<GatewayRequest>(100);

        let lnrpc =
            Self::create_lightning_client(lightning_mode.clone(), task_group.make_subgroup().await)
                .await?;

        let mut gw = Self {
            lnrpc,
            clients: Arc::new(RwLock::new(BTreeMap::new())),
            sender,
            receiver,
            task_group,
            channel_id_generator: AtomicU64::new(INITIAL_SCID),
            lightning_mode: Some(lightning_mode),
            fees,
            data_dir,
        };

        gw.load_clients().await?;

        Ok(gw)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn new_with_lightning_connection(
        lnrpc: Arc<dyn ILnRpcClient>,
        task_group: TaskGroup,
        fees: RoutingFees,
        data_dir: PathBuf,
    ) -> Result<Self> {
        // Create message channels for the webserver
        let (sender, receiver) = mpsc::channel::<GatewayRequest>(100);

        let mut gw = Self {
            lnrpc,
            clients: Arc::new(RwLock::new(BTreeMap::new())),
            sender,
            receiver,
            task_group,
            channel_id_generator: AtomicU64::new(INITIAL_SCID),
            lightning_mode: None,
            fees,
            data_dir,
        };

        gw.load_clients().await?;

        Ok(gw)
    }

    async fn create_lightning_client(
        mode: LightningMode,
        task_group: TaskGroup,
    ) -> Result<Arc<dyn ILnRpcClient>> {
        let lnrpc: Arc<dyn ILnRpcClient> = match mode {
            LightningMode::Cln { cln_extension_addr } => {
                info!(
                    "Gateway configured to connect to remote LnRpcClient at \n cln extension address: {:?} ",
                    cln_extension_addr
                );
                Arc::new(NetworkLnRpcClient::new(cln_extension_addr).await?)
            }
            LightningMode::Lnd {
                lnd_rpc_addr,
                lnd_tls_cert,
                lnd_macaroon,
            } => {
                info!(
                    "Gateway configured to connect to LND LnRpcClient at \n address: {:?},\n tls cert path: {:?},\n macaroon path: {} ",
                    lnd_rpc_addr, lnd_tls_cert, lnd_macaroon
                );
                Arc::new(
                    GatewayLndClient::new(lnd_rpc_addr, lnd_tls_cert, lnd_macaroon, task_group)
                        .await?,
                )
            }
        };

        Ok(lnrpc)
    }

    async fn build_client(&self, config: ClientConfig) -> anyhow::Result<fedimint_client::Client> {
        // create database
        let federation_id = config.federation_id;
        let db_path = self.data_dir.join(format!("{federation_id}.db"));
        let db = fedimint_rocksdb::RocksDb::open(db_path)?;

        // create module registry
        let mut registry = ClientModuleGenRegistry::new();
        registry.attach(MintClientGen);
        registry.attach(WalletClientGen);
        registry.attach(GatewayClientGen {
            lightning_client: self.lnrpc.clone(),
            // FIXME: don't hard-code
            fees: RoutingFees {
                base_msat: 0,
                proportional_millionths: 0,
            },
            timelock_delta: 10,
            api: Url::from_str("http://127.0.0.1:8175").unwrap(),
            mint_channel_id: 1,
        });

        let mut client_builder = ClientBuilder::default();
        client_builder.with_module_gens(registry);
        client_builder.with_primary_module(1);
        client_builder.with_config(config);
        client_builder.with_database(db);

        let mut tg = self.task_group.make_subgroup().await;
        client_builder
            // TODO: make this configurable?
            .build::<PlainRootSecretStrategy>(&mut tg)
            .await
    }

    // TODO: make this ClientConfig
    fn load_configs(&self) -> Result<Vec<GatewayClientConfig>> {
        Ok(std::fs::read_dir(&self.data_dir)
            .map_err(|e| GatewayError::Other(anyhow::Error::new(e)))?
            .filter_map(|file_res| {
                let file = file_res.ok()?;
                if !file.file_type().ok()?.is_file() {
                    return None;
                }

                if file
                    .path()
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .map(|ext| ext == "json")
                    .unwrap_or(false)
                {
                    Some(file)
                } else {
                    None
                }
            })
            .filter_map(|file| {
                // FIXME: handle parsing errors
                debug!("Trying to load config file {:?}", file.path());
                load_from_file(&file.path())
                    .map_err(|e| warn!("Could not parse config: {}", e))
                    .ok()
            })
            .collect())
    }

    async fn load_clients(&mut self) -> Result<()> {
        // TODO: do somehting with these route hints
        // Fetch route hints form the LN node
        // let mut num_retries = 0;
        // let route_hints = loop {
        //     let route_hints: Vec<RouteHint> = self
        //         .lnrpc
        //         .routehints()
        //         .await
        //         .expect("Could not fetch route hints")
        //         .try_into()
        //         .expect("Could not parse route hints");

        //     if !route_hints.is_empty() || num_retries == ROUTE_HINT_RETRIES {
        //         break route_hints;
        //     }

        //     info!(
        //         ?num_retries,
        //         "LN node returned no route hints, trying again in {}s",
        //         ROUTE_HINT_RETRY_SLEEP.as_secs()
        //     );
        //     num_retries += 1;
        //     task::sleep(ROUTE_HINT_RETRY_SLEEP).await;
        // };
        if let Ok(configs) = self.load_configs() {
            let mut next_channel_id = self.channel_id_generator.load(Ordering::SeqCst);

            for config in configs {
                let client = self.build_client(config.client_config).await?;
                // TODO: "join federation"
                // TODO: use route hints
                self.clients
                    .write()
                    .await
                    .insert(client.federation_id(), client);
                if config.mint_channel_id > next_channel_id {
                    next_channel_id = config.mint_channel_id + 1;
                }
            }
            self.channel_id_generator
                .store(next_channel_id, Ordering::SeqCst);
        } else {
            warn!("Could not load any previous federation configs");
        }
        Ok(())
    }

    // pub async fn load_actor(
    //     &mut self,
    //     client: Arc<GatewayClient>,
    //     route_hints: Vec<RouteHint>,
    // ) -> Result<GatewayActor> {
    //     let actor = GatewayActor::new(
    //         client.clone(),
    //         self.lnrpc.clone(),
    //         route_hints,
    //         self.task_group.clone(),
    //         GatewayRpcSender::new(self.sender.clone()),
    //     )
    //     .await?;

    //     self.actors.write().await.insert(
    //         client.config().client_config.federation_id.to_string(),
    //         actor.clone(),
    //     );
    //     Ok(actor)
    // }

    // async fn select_actor(&self, federation_id: FederationId) ->
    // Result<GatewayActor> {     self.actors
    //         .read()
    //         .await
    //         .get(&federation_id.to_string())
    //         .cloned()
    //         .ok_or(GatewayError::Other(anyhow::anyhow!(
    //             "No federation with id {}",
    //             federation_id.to_string()
    //         )))
    // }

    async fn handle_connect_federation(
        &mut self,
        payload: ConnectFedPayload,
        route_hints: Vec<RouteHint>,
        fees: RoutingFees,
    ) -> Result<FederationInfo> {
        let connect = WsClientConnectInfo::from_str(&payload.connect).map_err(|e| {
            GatewayError::Other(anyhow::anyhow!("Invalid federation member string {}", e))
        })?;

        todo!();

        // if let Ok(actor) = self.select_actor(connect.id).await {
        //     info!("Federation {} already connected", connect.id);
        //     return actor.get_info();
        // }

        // let GetNodeInfoResponse { pub_key, alias: _ } =
        // self.lnrpc.read().await.info().await?; let node_pub_key =
        // PublicKey::from_slice(&pub_key)     .map_err(|e|
        // GatewayError::Other(anyhow!("Invalid node pubkey {}", e)))?;

        // // The gateway deterministically assigns a channel id (u64) to each
        // federation // connected. TODO: explicitly handle the case
        // where the channel id // overflows
        // let channel_id = self.channel_id_generator.fetch_add(1,
        // Ordering::SeqCst);

        // let gw_client_cfg = self
        //     .client_builder
        //     .create_config(
        //         connect,
        //         channel_id,
        //         node_pub_key,
        //         self.module_gens.clone(),
        //         fees,
        //     )
        //     .await?;

        // let client = Arc::new(
        //     self.client_builder
        //         .build(
        //             gw_client_cfg.clone(),
        //             self.decoders.clone(),
        //             self.module_gens.clone(),
        //         )
        //         .await
        //         .expect("Failed to build gateway client"),
        // );

        // let actor = self
        //     .load_actor(client.clone(), route_hints)
        //     .await
        //     .map_err(|e| {
        //         GatewayError::Other(anyhow::anyhow!("Failed to connect
        // federation {}", e))     })?;

        // if let Err(e) = self.client_builder.save_config(client.config()) {
        //     warn!(
        //         "Failed to save default federation client configuration: {}",
        //         e
        //     );
        // }

        // let federation_info = actor.get_info()?;

        // Ok(federation_info)
    }

    async fn handle_get_info(&self, _payload: InfoPayload) -> Result<GatewayInfo> {
        todo!();
        // let actors = self.actors.read().await;
        // let mut federations: Vec<FederationInfo> = Vec::new();
        // for actor in actors.values() {
        //     federations.push(actor.get_info()?);
        // }

        // let ln_info = self.lnrpc.read().await.info().await?;

        // Ok(GatewayInfo {
        //     federations,
        //     version_hash: env!("CODE_VERSION").to_string(),
        //     lightning_pub_key: ln_info.pub_key.to_hex(),
        //     lightning_alias: ln_info.alias,
        //     fees: self.fees,
        // })
    }

    async fn handle_pay_invoice_msg(&self, payload: PayInvoicePayload) -> Result<Preimage> {
        todo!();
        // let PayInvoicePayload {
        //     federation_id,
        //     contract_id,
        // } = payload;

        // let actor = self.select_actor(federation_id).await?;
        // let (outpoint, preimage) = actor.pay_invoice(contract_id).await?;
        // actor
        //     .await_outgoing_contract_claimed(contract_id, outpoint)
        //     .await?;
        // Ok(preimage)
    }

    async fn handle_balance_msg(&self, payload: BalancePayload) -> Result<Amount> {
        todo!();
        // self.select_actor(payload.federation_id)
        //     .await?
        //     .get_balance()
        //     .await
    }

    async fn handle_address_msg(&self, payload: DepositAddressPayload) -> Result<Address> {
        todo!();
        // self.select_actor(payload.federation_id)
        //     .await?
        //     .get_deposit_address()
        //     .await
    }

    async fn handle_deposit_msg(&self, payload: DepositPayload) -> Result<TransactionId> {
        todo!();
        // let DepositPayload {
        //     txout_proof,
        //     transaction,
        //     federation_id,
        // } = payload;

        // self.select_actor(federation_id)
        //     .await?
        //     .deposit(txout_proof, transaction)
        //     .await
    }

    async fn handle_withdraw_msg(&self, payload: WithdrawPayload) -> Result<TransactionId> {
        todo!();
        // let WithdrawPayload {
        //     amount,
        //     address,
        //     federation_id,
        // } = payload;

        // self.select_actor(federation_id)
        //     .await?
        //     .withdraw(amount, address)
        //     .await
    }

    async fn handle_backup_msg(
        &self,
        BackupPayload { federation_id }: BackupPayload,
    ) -> Result<()> {
        todo!();
        // self.select_actor(federation_id).await?.backup().await
    }

    async fn handle_restore_msg(
        &self,
        RestorePayload { federation_id }: RestorePayload,
    ) -> Result<()> {
        todo!();
        // self.select_actor(federation_id)
        //     .await?
        //     .restore(self.task_group.make_subgroup().await)
        //     .await
    }

    async fn handle_lightning_reconnect(
        &mut self,
        payload: LightningReconnectPayload,
    ) -> Result<()> {
        todo!();
        // let LightningReconnectPayload { node_type } = payload;

        // let mut actors = self.actors.write().await;

        // // Stop all threads that are listening for HTLCs
        // tracing::info!("Stopping all HTLC subscription threads.");
        // for actor in actors.values_mut() {
        //     actor.stop_subscribing_htlcs().await?;
        // }

        // self.lnrpc = match node_type {
        //     Some(node_type) => {
        //         Self::create_lightning_client(node_type,
        // self.task_group.make_subgroup().await)             .await?
        //     }
        //     None => {
        //         // `lightning_mode` can be None during tests
        //         if self.lightning_mode.is_some() {
        //             Self::create_lightning_client(
        //                 self.lightning_mode.clone().unwrap(),
        //                 self.task_group.make_subgroup().await,
        //             )
        //             .await?
        //         } else {
        //             self.lnrpc.clone()
        //         }
        //     }
        // };

        // // Restart the subscription of HTLCs for each actor
        // tracing::info!("Restarting HTLC subscription threads.");

        // // Create a channel that will be used to shutdown the HTLC thread
        // for actor in actors.values_mut() {
        //     let (sender, receiver) = mpsc::channel::<Arc<AtomicBool>>(100);
        //     actor.route_htlcs(receiver).await?;
        //     actor.sender = sender;
        // }

        // Ok(())
    }

    pub async fn spawn_webserver(&self, listen: SocketAddr, password: String) {
        let sender = GatewayRpcSender::new(self.sender.clone());
        let tx = run_webserver(
            password,
            listen,
            sender,
            self.task_group.make_subgroup().await,
        )
        .await
        .expect("Failed to start webserver");

        // TODO: try to drive forward outgoing and incoming payments that were
        // interrupted
        let loop_ctrl = self.task_group.make_handle();
        let shutdown_sender = self.sender.clone();
        loop_ctrl
            .on_shutdown(Box::new(|| {
                Box::pin(async move {
                    // Send shutdown signal to the webserver
                    let _ = tx.send(());

                    // Send shutdown signal to the handler loop
                    let _ = shutdown_sender.send(GatewayRequest::Shutdown).await;
                })
            }))
            .await;
    }

    pub async fn run(mut self, loop_ctrl: TaskHandle) -> Result<()> {
        // Handle messages from webserver and plugin
        while let Some(msg) = self.receiver.recv().await {
            tracing::trace!("Gateway received message {:?}", msg);

            // Shut down main loop if requested
            if loop_ctrl.is_shutting_down() {
                break;
            }

            match msg {
                GatewayRequest::Info(inner) => {
                    inner
                        .handle(&mut self, |gateway, payload| {
                            gateway.handle_get_info(payload)
                        })
                        .await;
                }
                GatewayRequest::ConnectFederation(inner) => {
                    let route_hints: Vec<RouteHint> = self.lnrpc.routehints().await?.try_into()?;
                    let fees = self.fees;

                    inner
                        .handle(&mut self, |gateway, payload| {
                            gateway.handle_connect_federation(payload, route_hints.clone(), fees)
                        })
                        .await;
                }
                GatewayRequest::PayInvoice(inner) => {
                    inner
                        .handle(&mut self, |gateway, payload| {
                            gateway.handle_pay_invoice_msg(payload)
                        })
                        .await;
                }
                GatewayRequest::Balance(inner) => {
                    inner
                        .handle(&mut self, |gateway, payload| {
                            gateway.handle_balance_msg(payload)
                        })
                        .await;
                }
                GatewayRequest::DepositAddress(inner) => {
                    inner
                        .handle(&mut self, |gateway, payload| {
                            gateway.handle_address_msg(payload)
                        })
                        .await;
                }
                GatewayRequest::Deposit(inner) => {
                    inner
                        .handle(&mut self, |gateway, payload| {
                            gateway.handle_deposit_msg(payload)
                        })
                        .await;
                }
                GatewayRequest::Withdraw(inner) => {
                    inner
                        .handle(&mut self, |gateway, payload| {
                            gateway.handle_withdraw_msg(payload)
                        })
                        .await;
                }
                GatewayRequest::Backup(inner) => {
                    inner
                        .handle(&mut self, |gateway, payload| {
                            gateway.handle_backup_msg(payload)
                        })
                        .await;
                }
                GatewayRequest::Restore(inner) => {
                    inner
                        .handle(&mut self, |gateway, payload| {
                            gateway.handle_restore_msg(payload)
                        })
                        .await;
                }
                GatewayRequest::LightningReconnect(inner) => {
                    inner
                        .handle(&mut self, |gateway, payload| {
                            gateway.handle_lightning_reconnect(payload)
                        })
                        .await;
                }
                GatewayRequest::Shutdown => {
                    info!("Gatewayd received shutdown request");
                    break;
                }
            }
        }

        Ok(())
    }
}
