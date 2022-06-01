use crate::api::{ApiError, FederationApi};
use crate::clients::transaction::TransactionBuilder;
use crate::ln::gateway::LightningGateway;
use crate::ln::{LnClient, LnClientError};
use crate::mint::{MintClient, MintClientError, SpendableCoin};
use crate::wallet::{WalletClient, WalletClientError};
use crate::{api, OwnedClientContext};
use bitcoin::schnorr::KeyPair;
use bitcoin::{Address, Transaction as BitcoinTransaction};
use bitcoin_hashes::Hash;
use lightning::ln::PaymentSecret;
use lightning_invoice::{CreationError, Currency, Invoice, InvoiceBuilder};
use minimint::config::ClientConfig;
use minimint::modules::ln::contracts::incoming::{EncryptedPreimage, IncomingContractOffer};
use minimint::modules::ln::contracts::{ContractId, IdentifyableContract};
use minimint::modules::ln::{ContractAccount, ContractOrOfferOutput};
use minimint::modules::mint::tiered::coins::Coins;

use minimint::modules::wallet::txoproof::TxOutProof;

use minimint::modules::mint::BlindToken;
use minimint::transaction::{Input, Output, TransactionItem};
use minimint_api::db::batch::{Accumulator, BatchItem, DbBatch};
use minimint_api::db::Database;
use minimint_api::{Amount, TransactionId};
use minimint_api::{OutPoint, PeerId};
use rand::{CryptoRng, RngCore};
use secp256k1_zkp::{All, Secp256k1};
use std::time::Duration;
use thiserror::Error;

const TIMELOCK: u64 = 100;

pub struct UserClient {
    context: OwnedClientContext<ClientConfig>,
}

impl UserClient {
    pub fn new(cfg: ClientConfig, db: Box<dyn Database>, secp: Secp256k1<All>) -> Self {
        let api = api::HttpFederationApi::new(
            cfg.api_endpoints
                .iter()
                .enumerate()
                .map(|(id, url)| {
                    let peer_id = PeerId::from(id as u16); // FIXME: potentially wrong, currently works imo
                    let url = url.parse().expect("Invalid URL in config");
                    (peer_id, url)
                })
                .collect(),
        );
        Self::new_with_api(cfg, db, Box::new(api), secp)
    }

    pub fn new_with_api(
        config: ClientConfig,
        db: Box<dyn Database>,
        api: Box<dyn FederationApi>,
        secp: Secp256k1<All>,
    ) -> UserClient {
        UserClient {
            context: OwnedClientContext {
                config,
                db,
                api,
                secp,
            },
        }
    }

    fn ln_client(&self) -> LnClient {
        LnClient {
            context: self.context.borrow_with_module_config(|cfg| &cfg.ln),
        }
    }

    pub fn mint_client(&self) -> MintClient {
        MintClient {
            context: self.context.borrow_with_module_config(|cfg| &cfg.mint),
        }
    }

    fn wallet_client(&self) -> WalletClient {
        WalletClient {
            context: self.context.borrow_with_module_config(|cfg| &cfg.wallet),
            fee_consensus: self.context.config.fee_consensus.clone(), // TODO: remove or put into context
        }
    }

    pub async fn peg_in<R: RngCore + CryptoRng>(
        &self,
        txout_proof: TxOutProof,
        btc_transaction: BitcoinTransaction,
        mut rng: R,
    ) -> Result<TransactionId, ClientError> {
        let mut tx = TransactionBuilder::default();

        let (peg_in_key, peg_in_proof) = self
            .wallet_client()
            .create_pegin_input(txout_proof, btc_transaction)?;

        let amount = Amount::from_sat(peg_in_proof.tx_output().value)
            .saturating_sub(self.context.config.fee_consensus.fee_peg_in_abs);
        if amount == Amount::ZERO {
            return Err(ClientError::PegInAmountTooSmall);
        }

        tx.input(&mut vec![peg_in_key], Input::Wallet(Box::new(peg_in_proof)));

        self.submit_tx_with_change(tx, DbBatch::new(), &mut rng)
            .await
    }

    async fn submit_tx_with_change<R: RngCore + CryptoRng>(
        &self,
        tx: TransactionBuilder,
        mut batch: Accumulator<BatchItem>,
        mut rng: R,
    ) -> Result<TransactionId, ClientError> {
        let change = tx.change_required(&self.context.config.fee_consensus);
        let final_tx =
            self.mint_client()
                .finalize_change(change, batch.transaction(), tx, &mut rng);
        let txid = final_tx.tx_hash();
        let mint_tx_id = self.context.api.submit_transaction(final_tx).await?;
        // TODO: make check part of submit_transaction
        assert_eq!(
            txid, mint_tx_id,
            "Federation is faulty, returned wrong tx id."
        );

        self.context.db.apply_batch(batch).expect("DB error");
        Ok(txid)
    }

    /// Exchanges `coins` received from an untrusted third party for newly issued ones to prevent
    /// double spends. Users must ensure that the reissuance transaction is accepted before
    /// accepting `coins` as a valid payment.
    ///
    /// On success the out point of the newly issued e-cash tokens is returned. It can be used to
    /// easily poll the transaction status using [`MintClient::fetch_coins`] until it returns
    /// `Ok(())`, indicating we received our newly issued e-cash tokens.
    pub async fn reissue<R: RngCore + CryptoRng>(
        &self,
        coins: Coins<SpendableCoin>,
        mut rng: R,
    ) -> Result<OutPoint, ClientError> {
        let mut tx = TransactionBuilder::default();

        let (mut coin_keys, coin_input) = self.mint_client().create_coin_input_from_coins(coins)?;
        tx.input(&mut coin_keys, Input::Mint(coin_input));
        let txid = self
            .submit_tx_with_change(tx, DbBatch::new(), &mut rng)
            .await?;

        Ok(OutPoint { txid, out_idx: 0 })
    }

    pub async fn pay_for_coins<R: RngCore + CryptoRng>(
        &self,
        coins: Coins<BlindToken>,
        mut rng: R,
    ) -> Result<OutPoint, ClientError> {
        let mut batch = DbBatch::new();
        let mut tx = TransactionBuilder::default();

        let (mut coin_keys, coin_input) = self
            .mint_client()
            .create_coin_input(batch.transaction(), coins.amount())?;

        tx.input(&mut coin_keys, Input::Mint(coin_input));
        tx.output(Output::Mint(coins));
        let txid = self.submit_tx_with_change(tx, batch, &mut rng).await?;

        Ok(OutPoint { txid, out_idx: 0 })
    }

    pub fn receive_coins<R: RngCore + CryptoRng>(
        &self,
        amount: Amount,
        rng: R,
        create_tx: impl FnMut(Coins<BlindToken>) -> OutPoint,
    ) {
        let mut batch = DbBatch::new();

        self.mint_client()
            .create_coin_output(batch.transaction(), amount, rng, create_tx);

        self.context.db.apply_batch(batch).expect("DB error");
    }

    pub async fn peg_out<R: RngCore + CryptoRng>(
        &self,
        amt: bitcoin::Amount,
        address: Address,
        mut rng: R,
    ) -> Result<TransactionId, ClientError> {
        let mut batch = DbBatch::new();
        let mut tx = TransactionBuilder::default();

        let funding_amount = Amount::from(amt) + self.context.config.fee_consensus.fee_peg_out_abs;
        let (mut coin_keys, coin_input) = self
            .mint_client()
            .create_coin_input(batch.transaction(), funding_amount)?;
        let pegout_output = self.wallet_client().create_pegout_output(amt, address);

        tx.input(&mut coin_keys, Input::Mint(coin_input));
        tx.output(Output::Wallet(pegout_output));

        self.submit_tx_with_change(tx, batch, &mut rng).await
    }

    pub fn get_new_pegin_address<R: RngCore + CryptoRng>(&self, rng: R) -> Address {
        let mut batch = DbBatch::new();
        let address = self
            .wallet_client()
            .get_new_pegin_address(batch.transaction(), rng);
        self.context.db.apply_batch(batch).expect("DB error");
        address
    }

    pub fn select_and_spend_coins(
        &self,
        amount: Amount,
    ) -> Result<Coins<SpendableCoin>, MintClientError> {
        let mut batch = DbBatch::new();
        let coins = self
            .mint_client()
            .select_and_spend_coins(batch.transaction(), amount)?;
        self.context.db.apply_batch(batch).expect("DB error");
        Ok(coins)
    }

    /// Tries to fetch e-cash tokens from a certain out point. An error may just mean having queried
    /// the federation too early. Use [`MintClientError::is_retryable_fetch_coins`] to determine
    /// if the operation should be retried at a later time.
    pub async fn fetch_coins<'a>(&self, outpoint: OutPoint) -> Result<(), MintClientError> {
        let mut batch = DbBatch::new();
        self.mint_client()
            .fetch_coins(batch.transaction(), outpoint)
            .await?;
        self.context.db.apply_batch(batch).expect("DB error");
        Ok(())
    }

    pub async fn fetch_all_coins<'a>(&self) -> Result<Vec<TransactionId>, MintClientError> {
        let mut batch = DbBatch::new();
        let res = self
            .mint_client()
            .fetch_all_coins(batch.transaction())
            .await?;
        self.context.db.apply_batch(batch).expect("DB error");
        Ok(res)
    }

    pub fn coins(&self) -> Coins<SpendableCoin> {
        self.mint_client().coins()
    }

    pub async fn fund_outgoing_ln_contract<R: RngCore + CryptoRng>(
        &self,
        gateway: &LightningGateway,
        invoice: Invoice,
        mut rng: R,
    ) -> Result<ContractId, ClientError> {
        let mut batch = DbBatch::new();
        let mut tx = TransactionBuilder::default();

        let consensus_height = self.context.api.fetch_consensus_block_height().await?;
        let absolute_timelock = consensus_height + TIMELOCK;

        let contract = self
            .ln_client()
            .create_outgoing_output(
                batch.transaction(),
                invoice,
                gateway,
                absolute_timelock as u32,
                &mut rng,
            )
            .await?;
        let contract_id = match &contract {
            ContractOrOfferOutput::Contract(c) => c.contract.contract_id(),
            ContractOrOfferOutput::Offer(_) => {
                panic!()
            } // FIXME: impl TryFrom
        };
        let ln_output = Output::LN(contract);

        let amount = ln_output.amount();
        let (mut coin_keys, coin_input) = self
            .mint_client()
            .create_coin_input(batch.transaction(), amount)?;

        tx.input(&mut coin_keys, Input::Mint(coin_input));
        tx.output(ln_output);
        self.submit_tx_with_change(tx, batch, &mut rng).await?;

        Ok(contract_id)
    }

    pub async fn wait_contract(
        &self,
        contract: ContractId,
    ) -> Result<ContractAccount, ClientError> {
        loop {
            match self.ln_client().get_contract_account(contract).await {
                Ok(contract) => return Ok(contract),
                Err(LnClientError::ApiError(e)) => {
                    if e.is_retryable_fetch_coins() {
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    } else {
                        return Err(ClientError::MintApiError(e));
                    }
                }
                Err(e) => return Err(ClientError::LnClientError(e)),
            }
        }
    }

    pub async fn wait_contract_timeout(
        &self,
        contract: ContractId,
        timeout: Duration,
    ) -> Result<ContractAccount, ClientError> {
        tokio::time::timeout(timeout, self.wait_contract(contract))
            .await
            .map_err(|_| ClientError::WaitContractTimeout)?
    }

    pub async fn create_invoice_and_offer<R: RngCore + CryptoRng>(
        &self,
        amount: Amount,
        mut rng: R,
    ) -> Result<(KeyPair, Invoice), ClientError> {
        let (payment_keypair, payment_public_key) =
            self.context.secp.generate_schnorrsig_keypair(&mut rng);
        let raw_payment_secret = payment_public_key.serialize();
        let payment_hash = bitcoin::secp256k1::hashes::sha256::Hash::hash(&raw_payment_secret);
        let payment_secret = PaymentSecret(raw_payment_secret);

        // Temporary lightning node pubkey
        let (node_secret_key, node_public_key) = self.context.secp.generate_keypair(&mut rng);

        let invoice = InvoiceBuilder::new(Currency::Regtest)
            .amount_milli_satoshis(amount.milli_sat)
            .description("".into())
            .payment_hash(payment_hash)
            .payment_secret(payment_secret)
            .current_timestamp()
            .min_final_cltv_expiry(144)
            .payee_pub_key(node_public_key)
            .build_signed(|hash| self.context.secp.sign_recoverable(hash, &node_secret_key))?;

        let offer = IncomingContractOffer {
            amount,
            hash: payment_hash,
            encrypted_preimage: EncryptedPreimage::new(
                raw_payment_secret,
                &self.context.config.ln.threshold_pub_key,
            ),
        };
        let offer_output = ContractOrOfferOutput::Offer(offer.clone());
        let ln_output = Output::LN(offer_output);

        // There is no input here because this is just an announcement
        let mut tx = TransactionBuilder::default();
        tx.output(ln_output);
        self.submit_tx_with_change(tx, DbBatch::new(), &mut rng)
            .await?;
        Ok((payment_keypair, invoice))
    }

    pub async fn claim_incoming_contract(
        &self,
        contract_id: ContractId,
        keypair: KeyPair,
        mut rng: impl RngCore + CryptoRng,
    ) -> Result<TransactionId, ClientError> {
        // Lookup contract
        let contract = self.ln_client().get_incoming_contract(contract_id).await?;

        // Input claims this contract
        let mut tx = TransactionBuilder::default();
        tx.input(&mut vec![keypair], Input::LN(contract.claim()));
        self.submit_tx_with_change(tx, DbBatch::new(), &mut rng)
            .await
    }
}

#[derive(Error, Debug)]
pub enum ClientError {
    #[error("Error querying federation: {0}")]
    MintApiError(ApiError),
    #[error("Wallet client error: {0}")]
    WalletClientError(WalletClientError),
    #[error("Mint client error: {0}")]
    MintClientError(MintClientError),
    #[error("Lightning client error: {0}")]
    LnClientError(LnClientError),
    #[error("Peg-in amount must be greater than peg-in fee")]
    PegInAmountTooSmall,
    #[error("Timed out while waiting for contract to be accepted")]
    WaitContractTimeout,
    #[error("Error fetching offer")]
    FetchOfferError,
    #[error("Failed to create lightning invoice: {0}")]
    InvoiceError(CreationError),
}

impl From<ApiError> for ClientError {
    fn from(e: ApiError) -> Self {
        ClientError::MintApiError(e)
    }
}

impl From<WalletClientError> for ClientError {
    fn from(e: WalletClientError) -> Self {
        ClientError::WalletClientError(e)
    }
}

impl From<MintClientError> for ClientError {
    fn from(e: MintClientError) -> Self {
        ClientError::MintClientError(e)
    }
}

impl From<LnClientError> for ClientError {
    fn from(e: LnClientError) -> Self {
        ClientError::LnClientError(e)
    }
}

impl From<CreationError> for ClientError {
    fn from(e: CreationError) -> Self {
        ClientError::InvoiceError(e)
    }
}
