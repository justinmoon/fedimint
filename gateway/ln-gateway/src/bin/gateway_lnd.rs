use std::{path::PathBuf, sync::Arc};

use bitcoin::secp256k1::PublicKey;
use ln_gateway::{
    lnd::LndError, util::build_federation_client, GatewayRequest, LnGateway, LnGatewayError,
};
use tokio::sync::{mpsc, Mutex};

#[tokio::main]
async fn main() -> Result<(), LnGatewayError> {
    let mut args = std::env::args();

    if let Some(ref arg) = args.nth(1) {
        if arg.as_str() == "version-hash" {
            println!("{}", env!("GIT_HASH"));
            return Ok(());
        }
    }

    args.next().expect("not even zeroth arg given");
    let address = args
        .next()
        .expect("missing arguments: address, cert file, macaroon file, workdir");
    let cert_file = args
        .next()
        .expect("missing arguments: cert file, macaroon file, workdir");
    let macaroon_file = args
        .next()
        .expect("missing arguments: macaroon file, workdir");
    let work_dir = args.next().expect("missing argument: workdir");
    let work_dir = PathBuf::from(work_dir);

    // Connecting to LND requires only address, cert file, and macaroon file
    let mut ln_rpc = tonic_lnd::connect(address, cert_file, macaroon_file)
        .await
        .map_err(|e| {
            LnGatewayError::LightningRpcError(ln_gateway::ln::LightningError::LndError(
                LndError::ConnectError(e),
            ))
        })?;

    let info = ln_rpc
        // All calls require at least empty parameter
        .get_info(tonic_lnd::rpc::GetInfoRequest {})
        .await
        .expect("failed to get info")
        .into_inner();

    let pub_key: PublicKey = info.identity_pubkey.parse().expect("invalid pubkey"); // FIXME

    let (tx, rx): (mpsc::Sender<GatewayRequest>, mpsc::Receiver<GatewayRequest>) =
        mpsc::channel(100);

    // FIXME: do we need this???
    // let sender = GatewayRpcSender::new(tx.clone());

    // TODO: cli args to configure
    let bind_addr = "localhost:8080"
        .parse()
        .expect("Invalid gateway bind address");

    let federation_client = build_federation_client(pub_key, bind_addr, work_dir)?;
    let ln_rpc = Arc::new(Mutex::new(ln_rpc));
    let mut gateway = LnGateway::new(Arc::new(federation_client), ln_rpc, tx, rx, bind_addr);

    gateway.run().await.expect("gateway failed to run");
    Ok(())
}
