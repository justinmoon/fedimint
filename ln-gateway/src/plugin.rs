use crate::LnGateway;
use cln_plugin::Error;
use std::sync::Arc;

pub async fn buy_preimage(gateway: Arc<LnGateway>, v: serde_json::Value) -> Result<String, Error> {
    // TODO: actually fetch this from the preimage
    let preimage = String::from("0000000000000000000000000000000000000000000000000000000000000000");

    Ok(preimage)
}
