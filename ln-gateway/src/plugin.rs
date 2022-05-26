use crate::LnGateway;
use cln_plugin::Error;
use std::sync::Arc;
use std::sync::Mutex;

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

    // Submit bid

    // Wait for decryption

    // If preimage invalid, claw back funds and raise error
    // (or will the background thread find it???)

    // If preimage valid, return it so that plugin can complete htlc
    // TODO: save to db??

    // TODO: actually fetch this from the preimage
    // let preimage = String::from("0000000000000000000000000000000000000000000000000000000000000000");

    // gw.foo().await;

    Ok(preimage)
}
