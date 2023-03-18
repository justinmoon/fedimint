use std::fmt;

use async_trait::async_trait;
use bitcoin_hashes::sha256;
use fedimint_core::config::ModuleGenParams;
use fedimint_core::core::{Decoder, ModuleInstanceId, ModuleKind};
use fedimint_core::encoding::{Decodable, Encodable};
use fedimint_core::module::__reexports::serde_json;
use fedimint_core::module::{CommonModuleGen, ModuleCommon};
use fedimint_core::{plugin_types_trait_impl_common, Amount};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::config::DummyClientConfig;
use crate::serde_json::Value;
pub mod config;
pub mod db;

const KIND: ModuleKind = ModuleKind::from_static_str("dummy");

#[derive(Debug, Clone, Eq, PartialEq, Hash, Serialize, Deserialize, Encodable, Decodable)]
pub struct DummyConsensusItem;

#[derive(Debug)]
pub struct DummyCommonGen;

#[async_trait]
impl CommonModuleGen for DummyCommonGen {
    const KIND: ModuleKind = KIND;

    fn decoder() -> Decoder {
        DummyModuleTypes::decoder_builder().build()
    }

    fn hash_client_module(config: Value) -> anyhow::Result<sha256::Hash> {
        serde_json::from_value::<DummyClientConfig>(config)?.consensus_hash()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DummyConfigGenParams {
    pub important_param: u64,
}

impl ModuleGenParams for DummyConfigGenParams {
    const MODULE_NAME: &'static str = "dummy";
}

// #[derive(
//     Debug, Clone, Eq, PartialEq, Hash, Deserialize, Serialize, Encodable,
// Decodable, Default, )]
// pub struct DummyInput;
#[derive(Debug, Clone, Eq, PartialEq, Hash, Deserialize, Serialize, Encodable, Decodable)]
pub struct DummyInput {
    /// Block height of the lottery contract that we're claiming
    pub block_height: u64,
    /// Size of the pot they won
    pub amount: Amount,
}

impl fmt::Display for DummyInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DummyInput")
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, Deserialize, Serialize, Encodable, Decodable)]
pub struct DummyOutput {
    pub amount: fedimint_core::Amount,
    pub contract: LotteryContract,
}

impl fmt::Display for DummyOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DummyOutput")
    }
}
#[derive(Debug, Clone, Eq, PartialEq, Hash, Deserialize, Serialize, Encodable, Decodable)]
pub struct DummyOutputOutcome;

impl fmt::Display for DummyOutputOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DummyOutputOutcome")
    }
}

impl fmt::Display for DummyConsensusItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DummyOutputConfirmation")
    }
}

pub struct DummyModuleTypes;

impl ModuleCommon for DummyModuleTypes {
    type Input = DummyInput;
    type Output = DummyOutput;
    type OutputOutcome = DummyOutputOutcome;
    type ConsensusItem = DummyConsensusItem;
}

plugin_types_trait_impl_common!(
    DummyInput,
    DummyOutput,
    DummyOutputOutcome,
    DummyConsensusItem
);

#[derive(Debug, Clone, Eq, PartialEq, Hash, Error)]
pub enum DummyError {
    #[error("Something went wrong")]
    SomethingDummyWentWrong,
}

/// Contract for trust-minimized lotteries
///
/// User locks up funds associated with a future bitcoin block height and a
/// pubkey. All contracts for this block height are sorted lexigraphically by
/// pubkey, and the block hash of that block is interpreted as an integer and
/// modded by the number of bets to choose a winner
#[derive(Debug, Clone, Eq, PartialEq, Hash, Deserialize, Serialize, Encodable, Decodable)]
pub struct LotteryContract {
    /// Block height of the lottery
    pub block_height: u64,
    /// Public key of the user who is betting
    pub user_pubkey: secp256k1::XOnlyPublicKey,
    // TODO: amount
}
