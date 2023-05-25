use bitcoin_hashes::sha256;
use fedimint_client::sm::{OperationId, State, StateTransition};
use fedimint_client::DynGlobalClientContext;
use fedimint_core::encoding::{Decodable, Encodable};
use fedimint_core::Amount;
use fedimint_ln_common::api::LnFederationApi;
use fedimint_ln_common::contracts::outgoing::OutgoingContractAccount;
use fedimint_ln_common::contracts::{ContractId, FundedContract, Preimage};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::info;

use super::GatewayClientContext;
use crate::gatewaylnrpc::{PayInvoiceRequest, PayInvoiceResponse};

// TODO: Add diagram
#[derive(Debug, Clone, Eq, PartialEq, Decodable, Encodable)]
pub enum GatewayPayStates {
    PayInvoice(GatewayPayInvoice),
    Cancel,
    Preimage,
}

#[derive(Debug, Clone, Eq, PartialEq, Decodable, Encodable)]
pub struct GatewayPayCommon {
    pub operation_id: OperationId,
}

#[derive(Debug, Clone, Eq, PartialEq, Decodable, Encodable)]
pub struct GatewayPayStateMachine {
    pub common: GatewayPayCommon,
    pub state: GatewayPayStates,
}

impl State for GatewayPayStateMachine {
    type ModuleContext = GatewayClientContext;

    type GlobalContext = DynGlobalClientContext;

    fn transitions(
        &self,
        context: &Self::ModuleContext,
        global_context: &Self::GlobalContext,
    ) -> Vec<fedimint_client::sm::StateTransition<Self>> {
        match &self.state {
            GatewayPayStates::PayInvoice(gateway_pay_invoice) => gateway_pay_invoice.transitions(
                global_context.clone(),
                context.clone(),
                self.common.clone(),
            ),
            _ => {
                vec![]
            }
        }
    }

    fn operation_id(&self) -> fedimint_client::sm::OperationId {
        self.common.operation_id
    }
}

#[derive(Error, Debug, Serialize, Deserialize, Encodable, Decodable, Clone, Eq, PartialEq)]
pub enum GatewayPayError {
    #[error("OutgoingContract does not exist {contract_id}")]
    OutgoingContractDoesNotExist { contract_id: ContractId },
    #[error("Invalid OutgoingContract {contract_id}")]
    InvalidOutgoingContract { contract_id: ContractId },
    #[error("The contract is already cancelled and can't be processed by the gateway")]
    CancelledContract,
    #[error("The Account or offer is keyed to another gateway")]
    NotOurKey,
    #[error("Invoice is missing amount")]
    InvoiceMissingAmount,
    #[error("Outgoing contract is underfunded, wants us to pay {0}, but only contains {1}")]
    Underfunded(Amount, Amount),
    #[error("The contract's timeout is in the past or does not allow for a safety margin")]
    TimeoutTooClose,
    #[error("An error occurred while paying the lightning invoice.")]
    LightningPayError,
    #[error("Gateway could not retrieve metadata about the contract.")]
    MissingContractData,
}

#[derive(Debug, Clone, Eq, PartialEq, Decodable, Encodable)]
pub struct GatewayPayInvoice {
    pub contract_id: ContractId,
    pub timelock_delta: u64,
}

impl GatewayPayInvoice {
    fn transitions(
        &self,
        global_context: DynGlobalClientContext,
        context: GatewayClientContext,
        common: GatewayPayCommon,
    ) -> Vec<StateTransition<GatewayPayStateMachine>> {
        let timelock_delta = self.timelock_delta;
        vec![StateTransition::new(
            Self::await_buy_preimage(global_context.clone(), self.contract_id, context.clone()),
            move |_dbtx, result, _old_state| {
                info!("await_fetch_contract done: {result:?}");
                Box::pin(Self::transition_bought_preimage(
                    global_context.clone(),
                    result,
                    common.clone(),
                    timelock_delta,
                ))
            },
        )]
    }

    async fn await_buy_preimage(
        global_context: DynGlobalClientContext,
        contract_id: ContractId,
        context: GatewayClientContext,
    ) -> Result<Preimage, GatewayPayError> {
        let consensus_block_height = global_context
            .module_api()
            .fetch_consensus_block_height()
            .await
            .map_err(|_| GatewayPayError::TimeoutTooClose)?;

        if consensus_block_height.is_none() {
            return Err(GatewayPayError::MissingContractData);
        }

        info!("await_fetch_contract id={contract_id:?}");
        let account = global_context
            .module_api()
            .fetch_contract(contract_id)
            .await
            .map_err(|_| GatewayPayError::OutgoingContractDoesNotExist { contract_id })?;
        info!("fetched contract {account:?}");
        if let FundedContract::Outgoing(contract) = account.contract {
            let outgoing_contract_account = OutgoingContractAccount {
                amount: account.amount,
                contract,
            };

            let payment_parameters = Self::validate_outgoing_account(
                global_context,
                &outgoing_contract_account,
                context.redeem_key,
                context.timelock_delta,
                consensus_block_height.unwrap(),
            )
            .await?;
            return Self::await_buy_preimage_over_lightning(context, payment_parameters).await;
        }

        Err(GatewayPayError::OutgoingContractDoesNotExist { contract_id })
    }

    async fn await_buy_preimage_over_lightning(
        context: GatewayClientContext,
        buy_preimage: PaymentParameters,
    ) -> Result<Preimage, GatewayPayError> {
        let invoice = buy_preimage.invoice.clone();
        let max_delay = buy_preimage.max_delay;
        let max_fee_percent = buy_preimage.max_fee_percent();
        match context
            .lnrpc
            .pay(PayInvoiceRequest {
                invoice: invoice.to_string(),
                max_delay,
                max_fee_percent,
            })
            .await
        {
            Ok(PayInvoiceResponse { preimage, .. }) => {
                let slice: [u8; 32] = preimage.try_into().expect("Failed to parse preimage");
                Ok(Preimage(slice))
            }
            Err(e) => {
                info!("error paying lightning invoice {e:?}");
                Err(GatewayPayError::LightningPayError)
            }
        }
    }

    async fn transition_bought_preimage(
        global_context: DynGlobalClientContext,
        result: Result<Preimage, GatewayPayError>,
        common: GatewayPayCommon,
        timelock_delta: u64,
    ) -> GatewayPayStateMachine {
        match result {
            Ok(preimage) => GatewayPayStateMachine {
                common,
                state: GatewayPayStates::Preimage,
            },
            Err(_) => {
                info!("-> Cancel");
                return GatewayPayStateMachine {
                    common,
                    state: GatewayPayStates::Cancel,
                };
            }
        }
    }

    async fn validate_outgoing_account(
        global_context: DynGlobalClientContext,
        account: &OutgoingContractAccount,
        redeem_key: bitcoin::KeyPair,
        timelock_delta: u64,
        consensus_block_height: u64,
    ) -> Result<PaymentParameters, GatewayPayError> {
        let our_pub_key = secp256k1::XOnlyPublicKey::from_keypair(&redeem_key).0;

        if account.contract.cancelled {
            return Err(GatewayPayError::CancelledContract);
        }

        if account.contract.gateway_key != our_pub_key {
            return Err(GatewayPayError::NotOurKey);
        }

        let invoice = account.contract.invoice.clone();
        let invoice_amount = Amount::from_msats(
            invoice
                .amount_milli_satoshis()
                .ok_or(GatewayPayError::InvoiceMissingAmount)?,
        );

        if account.amount < invoice_amount {
            return Err(GatewayPayError::Underfunded(invoice_amount, account.amount));
        }

        let max_delay = (account.contract.timelock as u64)
            .checked_sub(consensus_block_height)
            .and_then(|delta| delta.checked_sub(timelock_delta));
        if max_delay.is_none() {
            return Err(GatewayPayError::TimeoutTooClose);
        }

        Ok(PaymentParameters {
            max_delay: max_delay.unwrap(),
            invoice_amount,
            max_send_amount: account.amount,
            payment_hash: *invoice.payment_hash(),
            invoice,
        })
    }
}

#[derive(Debug, Clone)]
pub struct PaymentParameters {
    max_delay: u64,
    invoice_amount: Amount,
    max_send_amount: Amount,
    payment_hash: sha256::Hash,
    invoice: lightning_invoice::Invoice,
}

impl PaymentParameters {
    fn max_fee_percent(&self) -> f64 {
        let max_absolute_fee = self.max_send_amount - self.invoice_amount;
        (max_absolute_fee.msats as f64) / (self.invoice_amount.msats as f64)
    }
}
