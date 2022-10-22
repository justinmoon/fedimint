use std::{net::SocketAddr, path::PathBuf};

use async_trait::async_trait;
use fedimint_server::modules::ln::contracts::Preimage;
use thiserror::Error;
use tokio::sync::Mutex;
use tracing::instrument;

use crate::{
    ln::{LightningError, LnRpc, LnRpcRef},
    rpc::GatewayRpcSender,
};

#[derive(Debug, Error)]
pub enum LndError {
    #[error("LND connect error {0}")]
    ConnectError(tonic_lnd::ConnectError),
    #[error("LND rpc error {0}")]
    RpcError(tonic_lnd::Error),
    #[error("No preimage")]
    NoPreimage,
}

#[async_trait]
impl LnRpc for Mutex<tonic_lnd::Client> {
    #[instrument(name = "LnRpc::pay", skip(self))]
    async fn pay(
        &self,
        invoice: &str,
        // TODO: use these
        _max_delay: u64,
        _max_fee_percent: f64,
    ) -> Result<Preimage, LightningError> {
        let send_response = self
            .lock()
            .await
            .send_payment_sync(tonic_lnd::rpc::SendRequest {
                payment_request: invoice.to_string(),
                ..Default::default()
            })
            .await
            .map_err(|e| LightningError::LndError(LndError::RpcError(e)))?
            .into_inner();

        if send_response.payment_preimage.is_empty() {
            return Err(LightningError::LndError(LndError::NoPreimage));
        };
        unimplemented!()
    }
}

// TODO: update arguments
// FIXME: is this the right error?
// pub async fn build_lnd_rpc(
//     _sender: GatewayRpcSender,
//     bind_addr: SocketAddr,
//     workdir: PathBuf,
// ) -> Result<LnRpcRef, LightningError> {

//     unimplemented!()
// }

pub fn spawn_htlc_interceptor(
    host: String,
    port: u32,
    cert_file_path: String,
    macaroon_file_path: String,
) {
    tokio::spawn(async move {
        // Connecting to LND requires only address, cert file, and macaroon file
        let mut client =
            tonic_openssl_lnd::connect_router(host, port, tls_cert_path, macaroon_path)
                .await
                .expect("failed to connect");

        let (tx, rx) = tokio::sync::mpsc::channel::<
            tonic_openssl_lnd::routerrpc::ForwardHtlcInterceptResponse,
        >(1024);
        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);

        let mut htlc_stream = client
            .htlc_interceptor(stream)
            .await
            .expect("Failed to call subscribe_invoices")
            .into_inner();

        while let Some(htlc) = htlc_stream
            .message()
            .await
            .expect("Failed to receive invoices")
        {
            println!("htlc {:?}", htlc);
            let lnd_htlc = LndHtlc {
                amount: htlc.incoming_amount_msat,
                payment_hash: htlc.payment_hash,
            };
            let preimage = plugin.state().send(htlc_accepted).await?;
            let preimage_string = preimage.to_public_key()?.to_string();
            // TODO: handle failure
            let response = tonic_openssl_lnd::routerrpc::ForwardHtlcInterceptResponse {
                incoming_circuit_key: htlc.incoming_circuit_key,
                action: 0,                 // this will claim the htlc
                preimage: preimage_string, // this would be for a real preimage
                failure_code: 0,
                failure_message: vec![],
            };
            tx.send(response).await.unwrap();
        }
    })
}
