use std::{net::SocketAddr, path::PathBuf};

use async_trait::async_trait;
use bitcoin_hashes::{sha256::Hash as Sha256Hash, Hash};
use fedimint_api::{db::SerializableDatabaseValue, Amount};
use fedimint_server::modules::ln::contracts::Preimage;
use thiserror::Error;
use tokio::{sync::Mutex, task::JoinHandle};
use tracing::instrument;

use crate::{
    ln::{LightningError, LnRpc, LnRpcRef},
    rpc::GatewayRpcSender,
    LndHtlc,
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

        // let preimage: Preimage = send_response.payment_preimage.into();

        // FIXME: try_into because payment_preimage is [u8], not [u8; 32] as desired
        Ok(Preimage(send_response.payment_preimage.try_into().unwrap()))
    }
}

pub fn spawn_htlc_interceptor(
    sender: GatewayRpcSender,
    host: String,
    port: u32,
    tls_cert_path: String,
    macaroon_path: String,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        // Connecting to LND requires only address, cert file, and macaroon file
        let mut client =
            tonic_openssl_lnd::connect_router(host, port, tls_cert_path, macaroon_path)
                .await
                .expect("failed to connect");

        println!("interceptor: connected");
        let (tx, rx) = tokio::sync::mpsc::channel::<
            tonic_openssl_lnd::routerrpc::ForwardHtlcInterceptResponse,
        >(1024);
        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);

        let mut htlc_stream = client
            .htlc_interceptor(stream)
            .await
            .expect("Failed to call subscribe_invoices")
            .into_inner();
        println!("interceptor: streaming");

        // loop {
        //     tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        //     println!("sleep");
        // }

        while let Some(htlc) = htlc_stream
            .message()
            .await
            .expect("Failed to receive invoices")
        {
            println!("htlc {:?}", htlc);
            let lnd_htlc = LndHtlc {
                amount: Amount::from_msat(htlc.incoming_amount_msat),
                payment_hash: Sha256Hash::from_slice(&htlc.payment_hash).unwrap(), // FIXME
            };
            let preimage = sender.send(lnd_htlc).await.unwrap(); // FIXME
            let preimage_bytes = preimage.to_public_key().unwrap().to_bytes(); // FIXME

            // TODO: handle failure
            let response = tonic_openssl_lnd::routerrpc::ForwardHtlcInterceptResponse {
                incoming_circuit_key: htlc.incoming_circuit_key,
                action: 0,                // this will claim the htlc
                preimage: preimage_bytes, // this would be for a real preimage
                failure_code: 0,
                failure_message: vec![],
            };
            tx.send(response).await.unwrap();
        }
    })
}
