use crate::ln::outgoing::OutgoingContractData;
use minimint::modules::ln::contracts::ContractId;
use minimint_api::db::DatabaseKeyPrefixConst;
use minimint_api::encoding::{Decodable, Encodable};

const DB_PREFIX_OUTGOING_PAYMENT: u8 = 0x40;
const DB_PREFIX_OFFER: u8 = 0x41;

#[derive(Debug, Encodable, Decodable)]
pub struct OutgoingPaymentKey(pub ContractId);

impl DatabaseKeyPrefixConst for OutgoingPaymentKey {
    const DB_PREFIX: u8 = DB_PREFIX_OUTGOING_PAYMENT;
    type Key = Self;
    type Value = OutgoingContractData;
}

#[derive(Debug, Encodable, Decodable)]
pub struct OutgoingPaymentKeyPrefix;

impl DatabaseKeyPrefixConst for OutgoingPaymentKeyPrefix {
    const DB_PREFIX: u8 = DB_PREFIX_OUTGOING_PAYMENT;
    type Key = OutgoingPaymentKey;
    type Value = OutgoingContractData;
}

// TODO: what is IncomingContractOffer? Do we need to implement it ourself in this crate as ^^ does
// Or can we import it?

// #[derive(Debug, Encodable, Decodable)]
// pub struct OfferKey(pub bitcoin_hashes::sha256::Hash);
//
// impl DatabaseKeyPrefixConst for OfferKey {
//     const DB_PREFIX: u8 = DB_PREFIX_OFFER;
//     type Key = Self;
//     type Value = IncomingContractOffer;
// }
//
// #[derive(Debug, Encodable, Decodable)]
// pub struct OfferKeyPrefix;
//
// impl DatabaseKeyPrefixConst for OfferKeyPrefix {
//     const DB_PREFIX: u8 = DB_PREFIX_OFFER;
//     type Key = OfferKey;
//     type Value = IncomingContractOffer;
// }
