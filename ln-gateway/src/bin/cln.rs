#[macro_use]
extern crate serde_json;
use cln_plugin::{options, Builder, Error, Plugin};
use ln_gateway::plugin::buy_preimage;
use ln_gateway::{LnGateway, LnGatewayConfig};
use minimint::config::load_from_file;
use minimint::modules::ln::contracts::ContractId;
use std::sync::Mutex;
use std::{path::PathBuf, sync::Arc};
use tide::Response;

use log::{debug, warn};
// use tracing::{debug, warn};
// use tracing_subscriber::EnvFilter;

pub async fn htlc_accepted_handler(
    p: Plugin<State>,
    v: serde_json::Value,
) -> Result<serde_json::Value, Error> {
    debug!("htlc_accepted observed");

    // TODO: buy this from federation
    // If the preimage matches, complete the payment
    // Check that the amount is matches
    // You've lost ecash tokens, but gained lightning btc
    // let preimage = "0000000000000000000000000000000000000000000000000000000000000000";
    let preimage = buy_preimage(p.state().gateway.clone(), v).await?;

    Ok(json!({
      "result": "resolve",
      "payment_key": preimage,
    }))
}

#[derive(Clone)]
pub struct State {
    // interior mutability needed b/c cln state must be created *before* plugin receives options needed to create LnGateway
    gateway: Arc<Mutex<Option<LnGateway>>>,
}

async fn pay_invoice(mut req: tide::Request<State>) -> tide::Result {
    let rng = rand::rngs::OsRng::new().unwrap();
    let contract: ContractId = req.body_json().await?;
    debug!("Received request to pay invoice of contract {}", contract);

    let State { gateway } = req.state();
    let gateway = gateway.lock().unwrap().take().unwrap();
    gateway
        .pay_invoice(contract, rng)
        .await
        .map_err(|e| {
            warn!("{:?}", e);
            tide::Error::from_debug(e)
        })
        .map(|()| Response::new(200))
}

async fn run_gateway(
    workdir: PathBuf,
    gateway_wrapper: Arc<Mutex<Option<LnGateway>>>,
) -> tide::Result<()> {
    // Give core-lightning some time to startup RPC socket (ln socket wasn't there ...)
    // FIXME: is there a better way?
    tokio::time::sleep(std::time::Duration::from_millis(3000)).await;

    let cfg_path = workdir.join("gateway.json");
    let db_path = workdir.join("gateway.db");
    let cfg: LnGatewayConfig = load_from_file(&cfg_path);
    let db = sled::open(&db_path)
        .unwrap()
        .open_tree("mint-client")
        .unwrap();

    let gateway = LnGateway::from_config(Box::new(db), cfg).await;

    // Update gateway in the plugin's state
    {
        let mut gateway_wrapper_writable = gateway_wrapper.lock().unwrap();
        *gateway_wrapper_writable = Some(gateway);
    }

    let state = State {
        gateway: gateway_wrapper,
    };

    let mut app = tide::with_state(state);
    app.at("/pay_invoice").post(pay_invoice);
    app.listen("27.0.0.1:8080").await?;

    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    // FIXME: kept getting `Error: attempted to set a logger after the logging system was already initialized`
    // tracing_subscriber::fmt()
    //     .with_env_filter(
    //         EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
    //     )
    //     .init();

    // Can't initialize gateway yet, but need to define it for the htlc_accepted_handler
    let gateway = Arc::new(Mutex::new(None));
    let state = State {
        gateway: gateway.clone(),
    };
    if let Some(plugin) = Builder::new(state, tokio::io::stdin(), tokio::io::stdout())
        .option(options::ConfigOption::new(
            "minimint-cfg",
            // FIXME: cln_plugin doesn't yet support optional parameters
            options::Value::String("default-dont-use".into()),
            "minimint config directory",
        ))
        .hook("htlc_accepted", htlc_accepted_handler)
        .start()
        .await?
    {
        let workdir = match plugin.option("minimint-cfg").expect("minimint-cfg missing") {
            options::Value::String(workdir) => {
                // FIXME: cln_plugin doesn't yet support optional parameters
                if &workdir == "default-done-use" {
                    panic!("minimint-cfg option missing")
                } else {
                    PathBuf::from(workdir)
                }
            }
            _ => unreachable!(),
        };
        tokio::spawn(run_gateway(workdir.clone(), gateway));
        plugin.join().await
    } else {
        Ok(())
    }
}
