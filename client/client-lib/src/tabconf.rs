use bitcoin::KeyPair;
use fedimint_api::db::DatabaseKeyPrefixConst;
use fedimint_api::encoding::Decodable;
use fedimint_api::encoding::Encodable;
use fedimint_api::module::TransactionItemAmount;
use fedimint_api::{Amount, FederationModule};
use fedimint_core::modules::tabconf::PlaceBet;
use fedimint_core::modules::tabconf::ResolvedBet;
use fedimint_core::modules::tabconf::TabconfConfig;
use fedimint_core::modules::tabconf::TabconfModule;
use rand::thread_rng;
use thiserror::Error;

use crate::utils::ClientContext;
use crate::{ApiError, ModuleClient};

// DB:
// - save PlaceBet?
const DB_PREFIX_PLACE_BET: u8 = 0x52;

#[derive(Debug, Encodable, Decodable)]
pub struct PlaceBetPrefix;

impl DatabaseKeyPrefixConst for PlaceBetPrefix {
    const DB_PREFIX: u8 = DB_PREFIX_PLACE_BET;
    type Key = PlaceBetPrefix;
    type Value = (KeyPair, PlaceBet);
}

/// Federation module client for the Tabconf module.
/// It can place and resolve bets
pub struct TabconfClient<'c> {
    pub config: &'c TabconfConfig, // FIXME: it's a little weird we're using module config for client config
    pub context: &'c ClientContext,
}

impl<'a> ModuleClient for TabconfClient<'a> {
    type Module = TabconfModule;

    fn input_amount(
        &self,
        input: &<Self::Module as FederationModule>::TxInput,
    ) -> TransactionItemAmount {
        TransactionItemAmount {
            amount: input.amount,
            fee: Amount::ZERO,
        }
    }

    fn output_amount(
        &self,
        _output: &<Self::Module as FederationModule>::TxOutput,
    ) -> TransactionItemAmount {
        TransactionItemAmount {
            amount: self.config.bet_size,
            fee: Amount::ZERO,
        }
    }
}

pub enum BetResult {
    Win,
    Lost,
    Pending,
}

impl<'c> TabconfClient<'c> {
    pub fn get_unresolved_bets(&self) -> Vec<(KeyPair, PlaceBet)> {
        self.context
            .db
            .find_by_prefix(&PlaceBetPrefix)
            .map(|res| res.expect("DB error").1)
            .collect()
    }
    pub async fn resolve_bet(&self, block_height: u64) -> Result<ResolvedBet> {
        self.context
            .api
            .winner(block_height)
            .await
            .map_err(TabconfClientError::ApiError)
    }
    pub fn create_bet_output<'a>(
        &'a self,
        resolve_consensus_height: u64,
        predicted_moscow_time: u64,
    ) -> PlaceBet {
        let kp = KeyPair::new(&self.context.secp, &mut thread_rng());
        // TODO: use correct amounts
        let output = PlaceBet {
            resolve_consensus_height,
            predicted_moscow_time,
            pub_key: kp.x_only_public_key().0,
        };

        // FIXME: should this be a transaction or something?
        self.context
            .db
            .insert_entry(&PlaceBetPrefix, &(kp, output.clone()))
            .unwrap();
        output
    }
}

type Result<T> = std::result::Result<T, TabconfClientError>;

#[derive(Error, Debug)]
pub enum TabconfClientError {
    #[error("Tabconf API error: {0}")]
    ApiError(ApiError),
}
