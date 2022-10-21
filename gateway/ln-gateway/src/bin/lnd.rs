///
/// cargo run --bin lnd http://localhost:11009 $PWD/lnd2/tls.cert $PWD/lnd2/data/chain/bitcoin/regtest/admin.macaroon
///
use thiserror::Error;

#[derive(Debug, Error)]
pub enum LndError {
    #[error("LND connect error {0}")]
    ConnectError(tonic_lnd::ConnectError),
    #[error("LND rpc error {0}")]
    RpcError(tonic_lnd::Error),
    #[error("No preimage")]
    NoPreimage,
}

#[tokio::main]
async fn main() -> Result<(), LndError> {
    let mut args = std::env::args_os();
    args.next().expect("not even zeroth arg given");
    let address = args
        .next()
        .expect("missing arguments: address, cert file, macaroon file, invoice");
    let cert_file = args
        .next()
        .expect("missing arguments: cert file, macaroon file, invoice");
    let macaroon_file = args
        .next()
        .expect("missing arguments: macaroon file, invoice");
    let invoice = args.next().expect("missing argument: invoice");
    let address = address.into_string().expect("address is not UTF-8");
    let invoice = invoice.into_string().expect("invoice is not UTF-8");

    // Connecting to LND requires only address, cert file, and macaroon file
    let mut client = tonic_lnd::connect(address, cert_file, macaroon_file)
        .await
        .map_err(LndError::ConnectError)?;

    let info = client
        // All calls require at least empty parameter
        .get_info(tonic_lnd::rpc::GetInfoRequest {})
        .await
        .map_err(LndError::RpcError)?;

    // We only print it here, note that in real-life code you may want to call `.into_inner()` on
    // the response to get the message.
    println!("{:#?}", info);

    // send payment
    // https://github.com/yzernik/squeakroad/blob/e0f4785616868874ee62aa6e6c29ffe6b2646f62/src/withdraw.rs#L151-L158

    // TODO: try SendPaymentV2
    let send_response = client
        .send_payment_sync(tonic_lnd::rpc::SendRequest {
            payment_request: invoice,
            ..Default::default()
        })
        .await
        .map_err(LndError::RpcError)?
        .into_inner();

    if send_response.payment_preimage.is_empty() {
        return Err(LndError::NoPreimage);
    };

    println!("IT WORKED {:?}", send_response.payment_preimage);

    Ok(())
}
