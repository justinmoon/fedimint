use fedimint_api::db::DatabaseKeyPrefixConst;
use fedimint_api::encoding::{Decodable, Encodable};
use fedimint_api::PeerId;
use secp256k1::XOnlyPublicKey;

use crate::ResolvedBet;

const DB_PREFIX_USER_BET_KEY: u8 = 0x50;
const DB_PREFIX_BET_RESOLUTION_KEY: u8 = 0x51;

/// Database key for a user bet, containing the height at which it will be resolved and the price
/// the user thinks will be closest to the actual BTC price. The value associated with the key is
/// the user's public key they can use to redeem their price in case they win
#[derive(Debug, Clone, Encodable, Decodable, Eq, PartialEq, Hash)]
pub struct UserBetKey {
    pub resolve_consensus_height: u64,
    /// aka sats per USD
    pub moscow_time: u64,
}

impl DatabaseKeyPrefixConst for UserBetKey {
    const DB_PREFIX: u8 = DB_PREFIX_USER_BET_KEY;
    type Key = Self;
    type Value = XOnlyPublicKey;
}

/// Database key prefix to query all bets that get resolved during the same block height
#[derive(Debug, Clone, Encodable, Decodable, Eq, PartialEq, Hash)]
pub struct UserBetKeyPrefix {
    resolve_consensus_height: u64,
}

impl DatabaseKeyPrefixConst for UserBetKeyPrefix {
    const DB_PREFIX: u8 = DB_PREFIX_USER_BET_KEY;
    type Key = UserBetKey;
    type Value = XOnlyPublicKey;
}

/// The key to the winner of a past, resolved bet
#[derive(Debug, Clone, Encodable, Decodable, Eq, PartialEq, Hash)]
pub struct BetResolutionKey {
    pub resolve_consensus_height: u64,
}

impl DatabaseKeyPrefixConst for BetResolutionKey {
    const DB_PREFIX: u8 = DB_PREFIX_BET_RESOLUTION_KEY;
    type Key = Self;
    type Value = ResolvedBet;
}

/// The key to the winner of a past, resolved bet
#[derive(Debug, Clone, Encodable, Decodable, Eq, PartialEq, Hash)]
pub struct BetResolutionKeyPrefix;

impl DatabaseKeyPrefixConst for BetResolutionKeyPrefix {
    const DB_PREFIX: u8 = DB_PREFIX_BET_RESOLUTION_KEY;
    type Key = BetResolutionKey;
    type Value = ResolvedBet;
}

///
#[derive(Debug, Clone, Encodable, Decodable, Eq, PartialEq, Hash)]
pub struct BetResolutionProposalKey {
    pub resolve_consensus_height: u64,
    pub peer: PeerId,
}
