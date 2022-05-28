pub mod outcome;
pub mod transaction;

use std::path::Path;

use minimint_ln::config::LightningModuleClientConfig;
use minimint_mint::config::MintClientConfig;
use minimint_wallet::config::WalletClientConfig;
use serde::{de::DeserializeOwned, Deserialize, Serialize};

pub fn load_from_file<T: DeserializeOwned>(path: &Path) -> T {
    let file = std::fs::File::open(path).expect("Can't read cfg file.");
    serde_json::from_reader(file).expect("Could not parse cfg file.")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientConfig {
    pub api_endpoints: Vec<String>,
    pub mint: MintClientConfig,
    pub wallet: WalletClientConfig,
    pub ln: LightningModuleClientConfig,
    pub fee_consensus: FeeConsensus,
}

// TODO: get rid of it here, modules should govern their oen fees
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeeConsensus {
    pub fee_coin_spend_abs: minimint_api::Amount,
    pub fee_peg_in_abs: minimint_api::Amount,
    pub fee_coin_issuance_abs: minimint_api::Amount,
    pub fee_peg_out_abs: minimint_api::Amount,
    pub fee_contract_input: minimint_api::Amount,
    pub fee_contract_output: minimint_api::Amount,
}
