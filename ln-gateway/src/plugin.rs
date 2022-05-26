use crate::LnGateway;
use cln_plugin::Error;
use std::sync::Arc;
use std::sync::Mutex;

// FIXME: we can probably just call gateway.buy_preimage() by itself ???
pub async fn buy_preimage(
    gateway: Arc<Mutex<Option<LnGateway>>>,
    v: serde_json::Value,
) -> Result<String, Error> {
    let gw = gateway.lock().unwrap().take().unwrap();

    // Is the pre-image for sale?
    log::info!("payment hash {:?}", v["htlc"]);
    let payment_hash: bitcoin_hashes::sha256::Hash =
        v["htlc"]["payment_hash"].as_str().unwrap().parse()?;
    let preimage = gw.buy_preimage(&payment_hash).await?;

    Ok(preimage)
}
