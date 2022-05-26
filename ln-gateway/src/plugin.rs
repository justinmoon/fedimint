use crate::LnGateway;
use cln_plugin::Error;
use std::sync::Arc;
use std::sync::Mutex;

pub async fn buy_preimage(
    gateway: Arc<Mutex<Option<LnGateway>>>,
    v: serde_json::Value,
) -> Result<String, Error> {
    let gw = gateway.lock().unwrap().take().unwrap();
    // TODO: actually fetch this from the preimage
    let preimage = String::from("0000000000000000000000000000000000000000000000000000000000000000");

    gw.foo().await;

    Ok(preimage)
}
