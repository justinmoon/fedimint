use clap::Parser;
use cln_plugin::{anyhow, options, Builder, Error, Plugin};
use ln_gateway::{LnGateway, LnGatewayConfig};
use minimint::config::load_from_file;
use minimint::modules::ln::contracts::ContractId;
use minimint_api::Amount;
use std::path::PathBuf;
use std::sync::Arc;
use tide::Response;
use tokio::io::{stdin, stdout};
use tokio::sync::Mutex;
use tracing::instrument;

#[derive(Clone)]
pub struct State {
    gateway: Arc<Mutex<Option<LnGateway>>>,
}

#[derive(Parser)]
struct Opts {
    workdir: PathBuf,
}

#[instrument(skip_all, err)]
async fn pay_invoice(mut req: tide::Request<State>) -> tide::Result {
    let rng = rand::rngs::OsRng::new().unwrap();
    let contract: ContractId = req.body_json().await?;
    let State { ref gateway } = req.state();

    let guard = gateway.lock().await;
    let gateway = match &*guard {
        Some(gateway) => gateway,
        None => return Err(tide::Error::from_str(500, "mutex empty")),
    };

    // debug!(%contract, "Received request to pay invoice");

    let result = gateway
        .pay_invoice(contract, rng)
        .await
        .map_err(tide::Error::from_debug)
        .map(|()| Response::new(200));
    gateway.await_contract_claimed(contract).await;
    result
}

async fn htlc_accepted_handler(
    plugin: Plugin<State>,
    value: serde_json::Value,
) -> Result<serde_json::Value, Error> {
    let handle = tokio::spawn(async move {
        let outcome: Result<serde_json::Value, anyhow::Error> =
            match _htlc_accepted_handler(plugin, value).await {
                Ok(v) => Ok(v),
                Err(e) => {
                    log::info!("htlc_accepted error {:?}", e);
                    Ok(serde_json::json!({
                        "result": "continue",
                    }))
                }
            };
        return outcome;
    });

    let outcome = handle.await?;
    Ok(outcome?)
}

async fn _htlc_accepted_handler(
    plugin: Plugin<State>,
    value: serde_json::Value,
) -> Result<serde_json::Value, Error> {
    log::info!("htlc_accepted observed {:?}", &value);

    // TODO: is there a better way to do this?
    let guard = plugin.state().gateway.lock().await;
    let gateway = match &*guard {
        Some(gateway) => gateway,
        None => {
            return Ok(serde_json::json!({
                "result": "continue",
            }))
        }
    };

    // Unwraps assume core-lightning doesn't give us malformed data ...
    let payment_hash: bitcoin_hashes::sha256::Hash = value["htlc"]["payment_hash"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let amount_msat_str: &str = value["htlc"]["amount"].as_str().unwrap();
    // This field looks like "1234msat" ... chop off "msat"
    let amount_str: &str = &amount_msat_str[0..amount_msat_str.len() - 4];
    let amount: Amount = amount_str.parse().unwrap();

    let rng = rand::rngs::OsRng::new().unwrap();
    let (txid, _) = gateway
        .buy_preimage_offer(&&payment_hash, &amount, rng)
        .await
        .map_err(|e| anyhow!("buy_preimage_offer error {:?}", e))?;

    log::info!("await decryption");
    let preimage = gateway
        .await_preimage_decryption(txid)
        .await
        .map_err(|e| anyhow!("await_preimage_decryption error {:?}", e))?;
    log::info!("decrypted {:?}", preimage.0.serialize());

    Ok(serde_json::json!({
      "result": "resolve",
      "payment_key": preimage,
    }))
}

async fn run_webserver(state: State) -> tide::Result<()> {
    let mut app = tide::with_state(state);
    app.at("/pay_invoice").post(pay_invoice);
    app.listen("127.0.0.1:8080").await?;

    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    let gateway_mutex = Arc::new(Mutex::new(None));
    let state = State {
        gateway: gateway_mutex.clone(),
    };
    if let Some(plugin) = Builder::new(state.clone(), stdin(), stdout())
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

        // Instantiate LnGateway and drop into mutex now that we know the config directory
        let cfg_path = workdir.join("gateway.json");
        let db_path = workdir.join("gateway.db");
        let cfg: LnGatewayConfig = load_from_file(&cfg_path);
        let db = sled::open(&db_path)
            .unwrap()
            .open_tree("mint-client")
            .unwrap();
        let gateway = LnGateway::from_config(Box::new(db), cfg).await;
        {
            let mut guard = gateway_mutex.lock().await;
            *guard = Some(gateway);
        }

        // Run gateway webserver and core-lightning plugin
        tokio::spawn(run_webserver(state));
        plugin.join().await
    } else {
        Ok(())
    }
}
