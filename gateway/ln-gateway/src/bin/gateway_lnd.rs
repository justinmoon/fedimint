use std::{path::PathBuf, sync::Arc};

use bitcoin::secp256k1::PublicKey;
use ln_gateway::{
    lnd::{spawn_htlc_interceptor, LndError},
    rpc::GatewayRpcSender,
    util::build_federation_client,
    GatewayRequest, LnGateway, LnGatewayError,
};
use tokio::sync::{mpsc, Mutex};

#[tokio::main]
async fn main() -> Result<(), LnGatewayError> {
    // FIXME: use clap ... this changes if run via `cargo run` or `./target/...`
    let mut args = std::env::args();

    // if let Some(ref arg) = args.nth(1) {
    //     if arg.as_str() == "version-hash" {
    //         println!("{}", env!("GIT_HASH"));
    //         return Ok(());
    //     }
    // }
    println!("interceptor: args {:?}", args);

    args.next().expect("not even zeroth arg given");
    let host = args
        .next()
        .expect("missing arguments: host, port, cert file, macaroon file, workdir");
    println!("host {}", host);
    let port = args
        .next()
        .expect("missing arguments: port, cert file, macaroon file, workdir");
    let tls_cert_path = args
        .next()
        .expect("missing arguments: cert file, macaroon file, workdir");
    let macaroon_path = args
        .next()
        .expect("missing arguments: macaroon file, workdir");
    let work_dir = args.next().expect("missing argument: workdir");
    let work_dir = PathBuf::from(work_dir);
    let port: u32 = port.parse().expect("port is not u32");

    // Connecting to LND requires only address, cert file, and macaroon file
    let address = format!("http://{}:{}", host, port);
    println!("interceptor: connecting");
    let mut ln_rpc = tonic_lnd::connect(address, tls_cert_path.clone(), macaroon_path.clone())
        .await
        .map_err(|e| {
            LnGatewayError::LightningRpcError(ln_gateway::ln::LightningError::LndError(
                LndError::ConnectError(e),
            ))
        })?;

    println!("interceptor: getinfo");
    let info = ln_rpc
        // All calls require at least empty parameter
        .get_info(tonic_lnd::rpc::GetInfoRequest {})
        .await
        .expect("failed to get info")
        .into_inner();

    let pub_key: PublicKey = info.identity_pubkey.parse().expect("invalid pubkey"); // FIXME

    let (tx, rx): (mpsc::Sender<GatewayRequest>, mpsc::Receiver<GatewayRequest>) =
        mpsc::channel(100);
    let sender = GatewayRpcSender::new(tx.clone());

    println!("interceptor: spawning");
    let handle = spawn_htlc_interceptor(sender, host, port, tls_cert_path, macaroon_path);

    // TODO: cli args to configure
    let bind_addr = "127.0.0.1:8080"
        .parse()
        .expect("Invalid gateway bind address");

    let federation_client = build_federation_client(pub_key, bind_addr, work_dir)?;
    let ln_rpc = Arc::new(Mutex::new(ln_rpc));
    let mut gateway = LnGateway::new(Arc::new(federation_client), ln_rpc, tx, rx, bind_addr);

    gateway.run().await.expect("gateway failed to run");
    Ok(())
}
