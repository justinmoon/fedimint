use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bitcoin::{Block, BlockHash, Network, Transaction};
use fedimint_api::{dyn_newtype_define, Feerate};
use thiserror::Error;
use tracing::info;

#[cfg(feature = "bitcoincore-rpc")]
pub mod bitcoincore_rpc;

#[derive(Error, Debug)]
pub enum Error {
    #[error("bitcoind Rpc error {0}")]
    Rpc(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Trait that allows interacting with the Bitcoin blockchain
///
/// Functions may panic if if the bitcoind node is not reachable.
#[async_trait]
pub trait IBitcoindRpc: Send + Sync {
    /// Returns the Bitcoin network the node is connected to
    async fn get_network(&self) -> Result<bitcoin::Network>;

    /// Returns the current block height
    async fn get_block_height(&self) -> Result<u64>;

    /// Returns the block hash at a given height
    ///
    /// # Panics
    /// If the node does not know a block for that height. Make sure to only query blocks of a
    /// height less or equal to the one returned by `Self::get_block_height`.
    ///
    /// While there is a corner case that the blockchain shrinks between these two calls (through on
    /// average heavier blocks on a fork) this is prevented by only querying hashes for blocks
    /// tailing the chain tip by a certain number of blocks.
    async fn get_block_hash(&self, height: u64) -> Result<BlockHash>;

    /// Returns the block with the given hash
    ///
    /// # Panics
    /// If the block doesn't exist.
    async fn get_block(&self, hash: &BlockHash) -> Result<bitcoin::Block>;

    /// Estimates the fee rate for a given confirmation target. Make sure that all federation
    /// members use the same algorithm to avoid widely diverging results. If the node is not ready
    /// yet to return a fee rate estimation this function returns `None`.
    async fn get_fee_rate(&self, confirmation_target: u16) -> Result<Option<Feerate>>;

    /// Submits a transaction to the Bitcoin network
    async fn submit_transaction(&self, transaction: Transaction) -> Result<()>;
}

dyn_newtype_define! {
    #[derive(Clone)]
    pub BitcoindRpc(Arc<IBitcoindRpc>)
}

/// Wrapper around [`IBitcoindRpc`] that will retry failed calls
#[derive(Debug)]
pub struct RetryClient<C> {
    inner: C,
    max_retries: u16,
    base_sleep: Duration,
}

impl<C> RetryClient<C> {
    pub fn new(inner: C) -> Self {
        Self {
            inner,
            max_retries: 10,
            base_sleep: Duration::from_millis(10),
        }
    }

    async fn retry_call<T, F, R>(&self, call_fn: F) -> Result<T>
    where
        F: Fn() -> R,
        R: Future<Output = Result<T>>,
    {
        let mut retries = 0;
        let mut fail_sleep = self.base_sleep;
        let ret = loop {
            match call_fn().await {
                Ok(ret) => {
                    break ret;
                }
                Err(e) => {
                    retries += 1;

                    if retries > self.max_retries {
                        return Err(e);
                    }

                    info!("Will retry rpc after {}ms", fail_sleep.as_millis());
                    std::thread::sleep(fail_sleep);
                    fail_sleep *= 2;
                }
            }
        };
        Ok(ret)
    }
}

#[async_trait]
impl<C> IBitcoindRpc for RetryClient<C>
where
    C: IBitcoindRpc,
{
    async fn get_network(&self) -> Result<Network> {
        self.retry_call(|| async { self.inner.get_network().await })
            .await
    }

    async fn get_block_height(&self) -> Result<u64> {
        self.retry_call(|| async { self.inner.get_block_height().await })
            .await
    }

    async fn get_block_hash(&self, height: u64) -> Result<BlockHash> {
        self.retry_call(|| async { self.inner.get_block_hash(height).await })
            .await
    }

    async fn get_block(&self, hash: &BlockHash) -> Result<Block> {
        self.retry_call(|| async { self.inner.get_block(hash).await })
            .await
    }

    async fn get_fee_rate(&self, confirmation_target: u16) -> Result<Option<Feerate>> {
        self.retry_call(|| async { self.inner.get_fee_rate(confirmation_target).await })
            .await
    }

    async fn submit_transaction(&self, transaction: Transaction) -> Result<()> {
        self.retry_call(|| async { self.inner.submit_transaction(transaction.clone()).await })
            .await
    }
}
