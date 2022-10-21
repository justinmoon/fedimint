use std::{net::SocketAddr, path::PathBuf, sync::Arc};

use async_trait::async_trait;
use fedimint_server::modules::ln::contracts::Preimage;
use secp256k1::PublicKey;
use thiserror::Error;

use crate::lnd::LndError;

#[async_trait]
pub trait LnRpc: Send + Sync + 'static {
    /// Attempt to pay an invoice and block till it succeeds, fails or times out
    async fn pay(
        &self,
        invoice: &str,
        max_delay: u64,
        max_fee_percent: f64,
    ) -> Result<Preimage, LightningError>;
}

#[derive(Clone)]
pub struct LnRpcRef {
    pub ln_rpc: Arc<dyn LnRpc>,
    pub bind_addr: SocketAddr,
    pub pub_key: PublicKey,
    pub work_dir: PathBuf,
}

// pub enum LightningError(pub Option<i32>);
#[derive(Debug, Error)]
pub enum LightningError {
    #[error("Rpc error")] // FIXME: display the i32
    ClnError(Option<i32>),
    #[error("Rpc error")] // FIXME: display the i32
    LndError(LndError),
}
