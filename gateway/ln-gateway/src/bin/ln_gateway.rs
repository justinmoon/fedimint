use std::sync::Arc;

use cln_plugin::Error;
use ln_gateway::{
    cln::build_cln_rpc, ln::LnRpcRef, rpc::GatewayRpcSender, util::build_federation_client,
    GatewayRequest, LnGateway,
};
use tokio::sync::mpsc;

#[tokio::main]
async fn main() -> Result<(), Error> {
    let mut args = std::env::args();

    if let Some(ref arg) = args.nth(1) {
        if arg.as_str() == "version-hash" {
            println!("{}", env!("GIT_HASH"));
            return Ok(());
        }
    }
    let (tx, rx): (mpsc::Sender<GatewayRequest>, mpsc::Receiver<GatewayRequest>) =
        mpsc::channel(100);

    let sender = GatewayRpcSender::new(tx.clone());
    let LnRpcRef {
        ln_rpc,
        bind_addr,
        pub_key,
        work_dir,
    } = build_cln_rpc(sender).await?;

    let federation_client = build_federation_client(pub_key, bind_addr, work_dir)?;
    let mut gateway = LnGateway::new(Arc::new(federation_client), ln_rpc, tx, rx, bind_addr);

    gateway.run().await.expect("gateway failed to run");
    Ok(())
}
