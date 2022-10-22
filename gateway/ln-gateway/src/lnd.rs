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
