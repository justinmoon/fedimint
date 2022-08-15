use anyhow::Result;
use bitcoin::hashes::hex::ToHex;
use bitcoin::Transaction;
use minimint_api::{Amount, OutPoint, TransactionId};
use minimint_core::modules::mint::tiered::coins::Coins;
use minimint_core::modules::wallet::txoproof::TxOutProof;
use mint_client::mint::{CoinFinalizationData, SpendableCoin};
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize)]
pub enum RpcResult {
    #[serde(rename = "success")]
    Success(serde_json::Value),
    #[serde(rename = "failure")]
    Failure(serde_json::Value),
}
/// struct to process wait_block_height request payload
#[derive(Deserialize, Serialize)]
pub struct WaitBlockHeightPayload {
    pub height: u64,
}

/// Struct used with the axum json-extractor to proccess the peg_in request payload
#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct PegInPayload {
    pub txout_proof: TxOutProof,
    pub transaction: Transaction,
}

#[derive(Serialize)]
pub struct InfoResponse {
    coins: Vec<CoinsByTier>,
    pending: PendingResponse,
}

impl InfoResponse {
    pub fn new(
        coins: Coins<SpendableCoin>,
        active_issuances: Vec<(OutPoint, CoinFinalizationData)>,
    ) -> Self {
        let info_coins: Vec<CoinsByTier> = coins
            .coins
            .iter()
            .map(|(tier, c)| CoinsByTier {
                quantity: c.len(),
                tier: tier.milli_sat,
            })
            .collect();
        Self {
            coins: info_coins,
            pending: PendingResponse::new(active_issuances),
        }
    }
}

#[derive(Serialize)]
pub struct PendingResponse {
    transactions: Vec<PendingTransaction>,
}

impl PendingResponse {
    pub fn new(active_issuances: Vec<(OutPoint, CoinFinalizationData)>) -> Self {
        let transactions: Vec<PendingTransaction> = active_issuances
            .iter()
            .map(|(out_point, cfd)| PendingTransaction {
                txid: out_point.txid.to_hex(),
                qty: cfd.coin_count(),
                value: cfd.coin_amount(),
            })
            .collect();
        Self { transactions }
    }
}

#[derive(Serialize)]
pub struct PegInAddressResponse {
    pub peg_in_address: bitcoin::Address,
}

#[derive(Serialize)]
pub struct PegInOutResponse {
    pub txid: TransactionId,
}

/// Holds a e-cash tier (msat by convention) and a quantity of coins
///
/// e.g { tier: 1000, quantity: 10 } means 10x coins worth 1000msat each
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CoinsByTier {
    tier: u64,
    quantity: usize,
}

/// Holds a pending transaction with the txid, the quantity of coins and the value
///
/// e.g { txid: xxx, qty: 10, value: 1 } is a pending transaction 'worth' 10btc
/// notice that this are ALL pending transactions not only the ['Accepted'](minimint_core::outcome::TransactionStatus) ones !
#[derive(Serialize)]
pub struct PendingTransaction {
    txid: String,
    qty: usize,
    value: Amount,
}

pub async fn call<P: Serialize + ?Sized>(params: &P, enpoint: &str) -> Result<RpcResult> {
    let client = reqwest::Client::new();

    let response = client
        .post(format!("http://127.0.0.1:8081{}", enpoint))
        .json(params)
        .send()
        .await?;

    Ok(response.json().await?)
}
