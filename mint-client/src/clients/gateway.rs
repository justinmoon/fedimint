use crate::api::{ApiError, FederationApi};
use crate::clients::gateway::db::{
    OutgoingPaymentClaimKey, OutgoingPaymentClaimKeyPrefix, OutgoingPaymentKey,
};
use crate::ln::outgoing::OutgoingContractAccount;
use crate::ln::{LnClient, LnClientError};
use crate::mint::{MintClient, MintClientError};
use crate::{api, OwnedClientContext};
use lightning_invoice::Invoice;
use minimint::config::ClientConfig;
use minimint::modules::ln::contracts::{
    incoming::{DecryptedPreimage, IncomingContract, IncomingContractOffer, Preimage},
    outgoing, Contract, ContractId, ContractOutcome, IdentifyableContract,
};
use minimint::modules::ln::{ContractOrOfferOutput, ContractOutput};
use minimint::outcome::{OutputOutcome, TransactionStatus};
use minimint::transaction::{agg_sign, Input, Output, Transaction, TransactionItem};
use minimint_api::db::batch::DbBatch;
use minimint_api::db::Database;
use minimint_api::{Amount, OutPoint, PeerId, TransactionId};
use rand::{CryptoRng, RngCore};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub struct GatewayClient {
    context: OwnedClientContext<GatewayClientConfig>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GatewayClientConfig {
    pub common: ClientConfig,
    #[serde(with = "serde_keypair")]
    pub redeem_key: secp256k1_zkp::schnorrsig::KeyPair,
    pub timelock_delta: u64,
}

#[derive(Debug)]
pub struct PaymentParameters {
    pub max_delay: u64,
    // FIXME: change to absolute fee to avoid rounding errors
    pub max_fee_percent: f64,
}

impl GatewayClient {
    pub fn new(cfg: GatewayClientConfig, db: Box<dyn Database>) -> Self {
        let api = api::HttpFederationApi::new(
            cfg.common
                .api_endpoints
                .iter()
                .enumerate()
                .map(|(id, url)| {
                    let peer_id = PeerId::from(id as u16); // FIXME: potentially wrong, currently works imo
                    let url = url.parse().expect("Invalid URL in config");
                    (peer_id, url)
                })
                .collect(),
        );
        Self::new_with_api(cfg, db, Box::new(api))
    }

    pub fn new_with_api(
        config: GatewayClientConfig,
        db: Box<dyn Database>,
        api: Box<dyn FederationApi>,
    ) -> GatewayClient {
        GatewayClient {
            context: OwnedClientContext {
                config,
                db,
                api,
                secp: secp256k1_zkp::Secp256k1::new(),
            },
        }
    }

    fn ln_client(&self) -> LnClient {
        LnClient {
            context: self.context.borrow_with_module_config(|cfg| &cfg.common.ln),
        }
    }

    fn mint_client(&self) -> MintClient {
        MintClient {
            context: self
                .context
                .borrow_with_module_config(|cfg| &cfg.common.mint),
        }
    }

    /// Fetch the specified outgoing payment contract account
    pub async fn fetch_outgoing_contract(
        &self,
        contract_id: ContractId,
    ) -> Result<OutgoingContractAccount> {
        self.ln_client()
            .get_outgoing_contract(contract_id)
            .await
            .map_err(GatewayClientError::LnClientError)
    }

    /// Check if we can claim the contract account and returns the max delay in blocks for how long
    /// other nodes on the route are allowed to delay the payment.
    pub async fn validate_outgoing_account(
        &self,
        account: &OutgoingContractAccount,
    ) -> Result<PaymentParameters> {
        let our_pub_key = secp256k1_zkp::schnorrsig::PublicKey::from_keypair(
            &self.context.secp,
            &self.context.config.redeem_key,
        );

        if account.contract.gateway_key != our_pub_key {
            return Err(GatewayClientError::NotOurKey);
        }

        let invoice: Invoice = account
            .contract
            .invoice
            .parse()
            .map_err(GatewayClientError::InvalidInvoice)?;
        let invoice_amount = Amount::from_msat(
            invoice
                .amount_milli_satoshis()
                .ok_or(GatewayClientError::InvoiceMissingAmount)?,
        );

        if account.amount < invoice_amount {
            return Err(GatewayClientError::Underfunded(
                invoice_amount,
                account.amount,
            ));
        }

        let max_absolute_fee = account.amount - invoice_amount;
        let max_fee_percent =
            (max_absolute_fee.milli_sat as f64) / (invoice_amount.milli_sat as f64);

        let consensus_block_height = self.context.api.fetch_consensus_block_height().await?;
        // Calculate max delay taking into account current consensus block height and our safety
        // margin.
        let max_delay = (account.contract.timelock as u64)
            .checked_sub(consensus_block_height)
            .and_then(|delta| delta.checked_sub(self.context.config.timelock_delta))
            .ok_or(GatewayClientError::TimeoutTooClose)?;

        Ok(PaymentParameters {
            max_delay,
            max_fee_percent,
        })
    }

    /// Save the details about an outgoing payment the client is about to process. This function has
    /// to be called prior to instructing the lightning node to pay the invoice since otherwise a
    /// crash could lead to loss of funds.
    ///
    /// Note though that extended periods of staying offline will result in loss of funds anyway if
    /// the client can not claim the respective contract in time.
    pub fn save_outgoing_payment(&self, contract: OutgoingContractAccount) {
        self.context
            .db
            .insert_entry(
                &db::OutgoingPaymentKey(contract.contract.contract_id()),
                &contract,
            )
            .expect("DB error");
    }

    /// Lists all previously saved transactions that have not been driven to completion so far
    pub fn list_pending_outgoing(&self) -> Vec<OutgoingContractAccount> {
        self.context
            .db
            .find_by_prefix(&db::OutgoingPaymentKeyPrefix)
            .map(|res| res.expect("DB error").1)
            .collect()
    }

    /// Abort payment if our node can't route it
    pub fn abort_outgoing_payment(&self, contract_id: ContractId) {
        // FIXME: implement abort by gateway to give funds back to user prematurely
        self.context
            .db
            .remove_entry(&db::OutgoingPaymentKey(contract_id))
            .expect("DB error");
    }

    /// Claim an outgoing contract after acquiring the preimage by paying the associated invoice and
    /// initiates e-cash issuances to receive the bitcoin from the contract (these still need to be
    /// fetched later to finalize them).
    ///
    /// Callers need to make sure that the contract can still be claimed by the gateway and has not
    /// timed out yet. Otherwise the transaction will fail.
    pub async fn claim_outgoing_contract(
        &self,
        contract_id: ContractId,
        preimage: [u8; 32],
        mut rng: impl RngCore + CryptoRng,
    ) -> Result<OutPoint> {
        let contract = self.ln_client().get_outgoing_contract(contract_id).await?;
        let input = Input::LN(contract.claim(outgoing::Preimage(preimage)));

        let (finalization_data, mint_output) = self
            .mint_client()
            .create_coin_output(input.amount(), &mut rng);
        let output = Output::Mint(mint_output);

        let inputs = vec![input];
        let outputs = vec![output];
        let txid = Transaction::tx_hash_from_parts(&inputs, &outputs);
        let signature = agg_sign(
            &[self.context.config.redeem_key],
            txid.as_hash(),
            &self.context.secp,
            &mut rng,
        );

        let out_point = OutPoint { txid, out_idx: 0 };
        let mut batch = DbBatch::new();
        self.mint_client().save_coin_finalization_data(
            batch.transaction(),
            out_point,
            finalization_data,
        );

        let transaction = Transaction {
            inputs,
            outputs,
            signature: Some(signature),
        };

        batch.autocommit(|batch| {
            batch.append_delete(OutgoingPaymentKey(contract_id));
            batch.append_insert(OutgoingPaymentClaimKey(contract_id), transaction.clone());
        });
        self.context.db.apply_batch(batch).expect("DB error");

        self.context.api.submit_transaction(transaction).await?;

        Ok(out_point)
    }

    pub async fn buy_preimage_offer(
        &self,
        payment_hash: &bitcoin_hashes::sha256::Hash,
        amount: &Amount,
        mut rng: impl RngCore + CryptoRng,
    ) -> Result<(minimint_api::TransactionId, ContractId)> {
        let mut batch = DbBatch::new();

        // See if there's an offer for this payment hash.
        let offers: Vec<IncomingContractOffer> = self.ln_client().get_offers().await?;
        let offer: IncomingContractOffer =
            match offers.into_iter().find(|o| &o.hash == payment_hash) {
                Some(o) => {
                    if &o.amount != amount || &o.hash != payment_hash {
                        return Err(GatewayClientError::InvalidOffer);
                    }
                    o
                }
                // FIXME: this should be an error LnClientError that we just bubble up
                None => return Err(GatewayClientError::NoOffer),
            };

        // Inputs
        let (coin_keys, coin_input) = self
            .mint_client()
            .create_coin_input(batch.transaction(), offer.amount)?;

        // Outputs
        let our_pub_key = secp256k1_zkp::schnorrsig::PublicKey::from_keypair(
            &self.context.secp,
            &self.context.config.redeem_key,
        );
        let contract = Contract::Incoming(IncomingContract {
            hash: offer.hash,
            encrypted_preimage: offer.encrypted_preimage.clone(),
            decrypted_preimage: DecryptedPreimage::Pending,
            gateway_key: our_pub_key,
        });
        let incoming_output =
            minimint::transaction::Output::LN(ContractOrOfferOutput::Contract(ContractOutput {
                amount: *amount,
                contract: contract.clone(),
            }));

        // Submit transaction
        let inputs = vec![minimint::transaction::Input::Mint(coin_input)];
        let outputs = vec![incoming_output];
        let txid = minimint::transaction::Transaction::tx_hash_from_parts(&inputs, &outputs);
        let signature = minimint::transaction::agg_sign(
            &coin_keys,
            txid.as_hash(),
            &self.context.secp,
            &mut rng,
        );
        let transaction = minimint::transaction::Transaction {
            inputs,
            outputs,
            signature: Some(signature),
        };
        let mint_tx_id = self.context.api.submit_transaction(transaction).await?;

        self.context.db.apply_batch(batch).expect("DB error");

        Ok((mint_tx_id, contract.contract_id()))
    }

    /// Claw back funds after outgoing contract that had invalid preimage
    pub async fn claim_incoming_contract(
        &self,
        contract_id: ContractId,
        mut rng: impl RngCore + CryptoRng,
    ) -> Result<OutPoint> {
        let contract_account = self.ln_client().get_incoming_account(contract_id).await?;

        // Input claims this contract
        let input = Input::LN(contract_account.claim());

        // Output pays us ecash tokens
        let (finalization_data, mint_output) = self
            .mint_client()
            .create_coin_output(input.amount(), &mut rng);
        let output = Output::Mint(mint_output);

        let inputs = vec![input];
        let outputs = vec![output];
        let txid = Transaction::tx_hash_from_parts(&inputs, &outputs);
        let signature = agg_sign(
            &[self.context.config.redeem_key],
            txid.as_hash(),
            &self.context.secp,
            &mut rng,
        );

        let out_point = OutPoint { txid, out_idx: 0 };
        let mut batch = DbBatch::new();
        self.mint_client().save_coin_finalization_data(
            batch.transaction(),
            out_point,
            finalization_data,
        );

        let transaction = Transaction {
            inputs,
            outputs,
            signature: Some(signature),
        };

        self.context.db.apply_batch(batch).expect("DB error");

        self.context.api.submit_transaction(transaction).await?;

        Ok(out_point)
    }

    /// Lists all claim transactions for outgoing contracts that we have submitted but were not part
    /// of the consensus yet.
    pub fn list_pending_claimed_outgoing(&self) -> Vec<ContractId> {
        self.context
            .db
            .find_by_prefix(&OutgoingPaymentClaimKeyPrefix)
            .map(|res| res.expect("DB error").0 .0)
            .collect()
    }

    pub async fn await_preimage_decryption(&self, txid: TransactionId) -> Result<Preimage> {
        loop {
            match self.context.api.fetch_tx_outcome(txid).await {
                Ok(status) => match status {
                    TransactionStatus::Accepted { outputs, .. } => {
                        match &outputs[0] {
                            OutputOutcome::LN(minimint::modules::ln::OutputOutcome::Contract {
                                outcome,
                                ..
                            }) => {
                                match outcome {
                                    ContractOutcome::Incoming(DecryptedPreimage::Some(
                                        preimage,
                                    )) => return Ok(preimage.clone()),
                                    ContractOutcome::Incoming(DecryptedPreimage::Pending) => {}
                                    ContractOutcome::Incoming(DecryptedPreimage::Invalid) => {
                                        return Err(GatewayClientError::InvalidPreimage)
                                    }
                                    _ => return Err(GatewayClientError::WrongContractType),
                                };
                            }
                            _ => return Err(GatewayClientError::WrongTransactionType),
                        };
                    }
                    TransactionStatus::Error(error) => {
                        return Err(GatewayClientError::InvalidTransaction(error))
                    }
                },
                Err(error) => return Err(GatewayClientError::MintApiError(error)),
            };
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        }
    }

    // TODO: improve error propagation on tx transmission
    /// Waits for a outgoing contract claim transaction to be confirmed and retransmits it
    /// periodically if this does not happen.
    ///
    /// # Panics
    /// * If the contract with the given ID isn't in the database. Only call this function with
    ///   pending claimed contracts returned by `list_pending_claimed_outgoing`.
    /// * If the task can not be completed in reasonable time meaning something went horribly
    ///   wrong (should not happen with an honest federation).
    pub async fn await_claimed_outgoing_accepted(&self, contract_id: ContractId) {
        let transaction: Transaction = self
            .context
            .db
            .get_value(&OutgoingPaymentClaimKey(contract_id))
            .expect("DB error")
            .expect("Contract not found");
        let txid = transaction.tx_hash();

        let await_confirmed = || async {
            loop {
                match self.context.api.fetch_tx_outcome(txid).await {
                    Ok(TransactionStatus::Accepted { .. }) => return,
                    Ok(TransactionStatus::Error(e)) => {
                        panic!("Transaction was rejected by federation: {}", e);
                    }
                    Err(e) => {
                        if e.is_retryable_fetch_coins() {
                            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                        } else {
                            // FIXME: maybe survive a few of these and only bail if the error remains, federation could be unreachable
                            panic!(
                                "Federation returned error when fetching tx outcome: {:?}",
                                e
                            );
                        }
                    }
                }
            }
        };

        let timeout = tokio::time::Duration::from_secs(10);
        for _ in 0..6 {
            tokio::select! {
                () = await_confirmed() => {
                    // We remove the entry that indicates we are still waiting for transaction
                    // confirmation. This does not mean we are finished yet. As a last step we need
                    // to fetch the blind signatures for the newly issued tokens, but as long as the
                    // federation is honest as a whole they will produce the signatures, so we don't
                    // have to worry about that.
                    self.context
                        .db
                        .remove_entry(&OutgoingPaymentClaimKey(contract_id))
                        .expect("DB error");
                    return;
                },
                _ = tokio::time::sleep(timeout) => {
                    self.context
                        .api
                        .submit_transaction(transaction.clone())
                        .await
                        .expect("Error submitting transaction");
                }
            };
        }

        panic!(
            "Could not get claim transaction for contract {} finalized",
            contract_id
        );
    }

    pub fn list_fetchable_coins(&self) -> Vec<OutPoint> {
        self.mint_client().list_active_issuances()
    }

    /// Tries to fetch e-cash tokens from a certain out point. An error may just mean having queried
    /// the federation too early. Use [`MintClientError::is_retryable_fetch_coins`] to determine
    /// if the operation should be retried at a later time.
    pub async fn fetch_coins<'a>(
        &self,
        outpoint: OutPoint,
    ) -> std::result::Result<(), MintClientError> {
        let mut batch = DbBatch::new();
        self.mint_client()
            .fetch_coins(batch.transaction(), outpoint)
            .await?;
        self.context.db.apply_batch(batch).expect("DB error");
        Ok(())
    }
}

type Result<T> = std::result::Result<T, GatewayClientError>;

#[derive(Error, Debug)]
pub enum GatewayClientError {
    #[error("Error querying federation: {0}")]
    MintApiError(ApiError),
    #[error("Mint client error: {0}")]
    MintClientError(MintClientError),
    #[error("Lightning client error: {0}")]
    LnClientError(LnClientError),
    #[error("The Account or offer is keyed to another gateway")]
    NotOurKey,
    #[error("Can't parse contract's invoice: {0:?}")]
    InvalidInvoice(lightning_invoice::ParseOrSemanticError),
    #[error("Invoice is missing amount")]
    InvoiceMissingAmount,
    #[error("Outgoing contract is underfunded, wants us to pay {0}, but only contains {1}")]
    Underfunded(Amount, Amount),
    #[error("The contract's timeout is in the past or does not allow for a safety margin")]
    TimeoutTooClose,
    #[error("No offer")]
    NoOffer,
    #[error("Invalid offer")]
    InvalidOffer,
    #[error("Wrong contract type")]
    WrongContractType,
    #[error("Wrong transaction type")]
    WrongTransactionType,
    #[error("Invalid transaction {0}")]
    InvalidTransaction(String),
    #[error("Invalid preimage")]
    InvalidPreimage,
}

impl From<LnClientError> for GatewayClientError {
    fn from(e: LnClientError) -> Self {
        GatewayClientError::LnClientError(e)
    }
}

impl From<ApiError> for GatewayClientError {
    fn from(e: ApiError) -> Self {
        GatewayClientError::MintApiError(e)
    }
}

impl From<MintClientError> for GatewayClientError {
    fn from(e: MintClientError) -> Self {
        GatewayClientError::MintClientError(e)
    }
}

mod db {
    use crate::ln::outgoing::OutgoingContractAccount;
    use minimint::modules::ln::contracts::ContractId;
    use minimint::transaction::Transaction;
    use minimint_api::db::DatabaseKeyPrefixConst;
    use minimint_api::encoding::{Decodable, Encodable};

    const DB_PREFIX_OUTGOING_PAYMENT: u8 = 0x50;
    const DB_PREFIX_OUTGOING_PAYMENT_CLAIM: u8 = 0x51;

    #[derive(Debug, Encodable, Decodable)]
    pub struct OutgoingPaymentKey(pub ContractId);

    impl DatabaseKeyPrefixConst for OutgoingPaymentKey {
        const DB_PREFIX: u8 = DB_PREFIX_OUTGOING_PAYMENT;
        type Key = Self;
        type Value = OutgoingContractAccount;
    }

    #[derive(Debug, Encodable, Decodable)]
    pub struct OutgoingPaymentKeyPrefix;

    impl DatabaseKeyPrefixConst for OutgoingPaymentKeyPrefix {
        const DB_PREFIX: u8 = DB_PREFIX_OUTGOING_PAYMENT;
        type Key = OutgoingPaymentKey;
        type Value = OutgoingContractAccount;
    }

    #[derive(Debug, Encodable, Decodable)]
    pub struct OutgoingPaymentClaimKey(pub ContractId);

    impl DatabaseKeyPrefixConst for OutgoingPaymentClaimKey {
        const DB_PREFIX: u8 = DB_PREFIX_OUTGOING_PAYMENT_CLAIM;
        type Key = Self;
        type Value = Transaction;
    }

    #[derive(Debug, Encodable, Decodable)]
    pub struct OutgoingPaymentClaimKeyPrefix;

    impl DatabaseKeyPrefixConst for OutgoingPaymentClaimKeyPrefix {
        const DB_PREFIX: u8 = DB_PREFIX_OUTGOING_PAYMENT_CLAIM;
        type Key = OutgoingPaymentClaimKey;
        type Value = Transaction;
    }
}

pub mod serde_keypair {
    use secp256k1_zkp::schnorrsig::KeyPair;
    use secp256k1_zkp::SecretKey;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    #[allow(missing_docs)]
    pub fn serialize<S>(key: &KeyPair, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        SecretKey::from_keypair(key).serialize(serializer)
    }

    #[allow(missing_docs)]
    pub fn deserialize<'de, D>(deserializer: D) -> Result<KeyPair, D::Error>
    where
        D: Deserializer<'de>,
    {
        let secret_key = SecretKey::deserialize(deserializer)?;

        Ok(KeyPair::from_secret_key(
            secp256k1_zkp::SECP256K1,
            secret_key,
        ))
    }
}
