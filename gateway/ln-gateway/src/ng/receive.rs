use std::ops::Add;
use std::sync::Arc;
use std::time::Duration;

use bitcoin_hashes::{sha256, Hash};
use fedimint_client::sm::{ClientSMDatabaseTransaction, OperationId, State, StateTransition};
use fedimint_client::transaction::{ClientInput, ClientOutput, TxSubmissionError};
use fedimint_client::DynGlobalClientContext;
use fedimint_core::api::GlobalFederationApi;
use fedimint_core::core::{Decoder, OutputOutcome};
use fedimint_core::encoding::{Decodable, Encodable};
use fedimint_core::task::{sleep, timeout};
use fedimint_core::{Amount, OutPoint, TransactionId};
use fedimint_ln_client::api::LnFederationApi;
use fedimint_ln_common::contracts::incoming::{
    IncomingContract, IncomingContractAccount, IncomingContractOffer,
};
use fedimint_ln_common::contracts::{Contract, ContractId, DecryptedPreimage, Preimage};
use fedimint_ln_common::{ContractOutput, LightningInput, LightningOutput, LightningOutputOutcome};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::time::Instant;
use tracing::info;

use super::{GatewayClientContext, GatewayClientStateMachines};
use crate::gatewaylnrpc::complete_htlcs_request::{Action, Settle};
use crate::gatewaylnrpc::{
    route_htlc_request, CompleteHtlcsRequest, RouteHtlcRequest, SubscribeInterceptHtlcsResponse,
};

#[derive(Error, Debug, Serialize, Deserialize, Encodable, Decodable, Clone, Eq, PartialEq)]
pub enum ReceiveError {
    #[error("Violated fee policy")]
    ViolatedFeePolicy,
    #[error("Invalid offer")]
    InvalidOffer,
    #[error("Timeout")]
    Timeout,
    #[error("Fetch contract error")]
    FetchContractError,
    #[error("Incoming contract error")]
    IncomingContractError,
    #[error("Invalid preimage")]
    InvalidPreimage,
    #[error("Output outcome error")]
    OutputOutcomeError,
    #[error("Route htlc error")]
    RouteHtlcError,
    #[error("Incoming contract not found")]
    IncomingContractNotFound,
}

#[derive(Debug, Clone, Eq, PartialEq, Decodable, Encodable)]
pub enum GatewayReceiveStates {
    HtlcIntercepted(HtlcIntercepted),
    Funding(AwaitingContractAcceptance),
    Preimage(PreimageState),
    Settled,
    Refund(RefundState),
    // TODO: include txid
    RefundSuccess(RefundSuccessState),
    RefundError(RefundErrorState),
    Failed,
}

#[derive(Debug, Clone, Eq, PartialEq, Decodable, Encodable)]
pub struct GatewayReceiveCommon {
    pub operation_id: OperationId,
}

#[derive(Debug, Clone, Eq, PartialEq, Decodable, Encodable)]
pub struct GatewayReceiveStateMachine {
    pub common: GatewayReceiveCommon,
    pub state: GatewayReceiveStates,
}

impl State for GatewayReceiveStateMachine {
    type ModuleContext = GatewayClientContext;

    type GlobalContext = DynGlobalClientContext;

    fn transitions(
        &self,
        context: &Self::ModuleContext,
        global_context: &Self::GlobalContext,
    ) -> Vec<fedimint_client::sm::StateTransition<Self>> {
        match &self.state {
            GatewayReceiveStates::HtlcIntercepted(state) => {
                state.transitions(global_context.clone(), context.clone(), self.common.clone())
            }
            GatewayReceiveStates::Funding(state) => {
                state.transitions(&global_context, &context, &self.common)
            }
            GatewayReceiveStates::Refund(state) => state.transitions(&self.common, &global_context),
            _ => {
                vec![]
            }
        }
    }

    fn operation_id(&self) -> fedimint_client::sm::OperationId {
        self.common.operation_id
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Decodable, Encodable)]
pub struct Htlc {
    /// The HTLC payment hash.
    pub payment_hash: sha256::Hash,
    /// The incoming HTLC amount in millisatoshi.
    pub incoming_amount_msat: Amount,
    /// The outgoing HTLC amount in millisatoshi
    pub outgoing_amount_msat: Amount,
    /// The incoming HTLC expiry
    pub incoming_expiry: u32,
    /// The short channel id of the HTLC.
    pub short_channel_id: u64,
    /// The id of the incoming channel
    pub incoming_chan_id: u64,
    /// The index of the incoming htlc in the incoming channel
    pub htlc_id: u64,
}

impl From<SubscribeInterceptHtlcsResponse> for Htlc {
    fn from(s: SubscribeInterceptHtlcsResponse) -> Self {
        Self {
            // FIXME: unwrap
            payment_hash: sha256::Hash::from_slice(&s.payment_hash).unwrap(),
            incoming_amount_msat: Amount::from_msats(s.incoming_amount_msat),
            outgoing_amount_msat: Amount::from_msats(s.outgoing_amount_msat),
            incoming_expiry: s.incoming_expiry,
            short_channel_id: s.short_channel_id,
            incoming_chan_id: s.incoming_chan_id,
            htlc_id: s.htlc_id,
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Decodable, Encodable)]
pub struct HtlcIntercepted {
    pub htlc: Htlc,
}

impl HtlcIntercepted {
    fn transitions(
        &self,
        global_context: DynGlobalClientContext,
        context: GatewayClientContext,
        common: GatewayReceiveCommon,
    ) -> Vec<StateTransition<GatewayReceiveStateMachine>> {
        let htlc = self.htlc.clone();
        vec![StateTransition::new(
            Self::intercept_htlc(global_context.clone(), context.clone(), self.htlc.clone()),
            move |dbtx, result, _old_state| {
                info!("await_buy_preimage done: {result:?}");
                Box::pin(Self::transition_intercept_htlc(
                    global_context.clone(),
                    dbtx,
                    result,
                    common.clone(),
                    context.clone(),
                    htlc.clone(),
                ))
            },
        )]
    }

    async fn intercept_htlc(
        global_context: DynGlobalClientContext,
        context: GatewayClientContext,
        htlc: Htlc,
    ) -> Result<IncomingContractOffer, ReceiveError> {
        // FIXME: timeout?
        let offer: IncomingContractOffer = global_context
            .module_api()
            .fetch_offer(htlc.payment_hash)
            .await
            .map_err(|_| ReceiveError::FetchContractError)?; // FIXME: don't swallow this error

        if offer.amount > htlc.outgoing_amount_msat {
            return Err(ReceiveError::ViolatedFeePolicy);
        }
        if offer.hash != htlc.payment_hash {
            return Err(ReceiveError::InvalidOffer);
        }
        Ok(offer)
    }

    async fn transition_intercept_htlc(
        global_context: DynGlobalClientContext,
        dbtx: &mut ClientSMDatabaseTransaction<'_, '_>,
        result: Result<IncomingContractOffer, ReceiveError>,
        common: GatewayReceiveCommon,
        context: GatewayClientContext,
        htlc: Htlc,
    ) -> GatewayReceiveStateMachine {
        match result {
            Ok(offer) => {
                // Outputs
                let our_pub_key =
                    secp256k1_zkp::XOnlyPublicKey::from_keypair(&context.redeem_key).0;
                let contract = Contract::Incoming(IncomingContract {
                    hash: offer.hash,
                    encrypted_preimage: offer.encrypted_preimage.clone(),
                    decrypted_preimage: DecryptedPreimage::Pending,
                    gateway_key: our_pub_key,
                });
                let incoming_output = LightningOutput::Contract(ContractOutput {
                    amount: offer.amount,
                    contract: contract.clone(),
                });
                let client_output = ClientOutput::<LightningOutput, GatewayClientStateMachines> {
                    output: incoming_output,
                    state_machines: Arc::new(|_, _| vec![]),
                };
                // FIXME: unwrap
                let (txid, change) = global_context
                    .fund_output(dbtx, client_output)
                    .await
                    .unwrap();
                GatewayReceiveStateMachine {
                    common,
                    state: GatewayReceiveStates::Funding(AwaitingContractAcceptance {
                        txid,
                        change,
                        htlc,
                    }),
                }
            }
            Err(e) => GatewayReceiveStateMachine {
                common,
                state: GatewayReceiveStates::Failed,
            },
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Decodable, Encodable)]
pub struct AwaitingContractAcceptance {
    txid: TransactionId,
    change: Option<OutPoint>,
    htlc: Htlc,
}

impl AwaitingContractAcceptance {
    fn transitions(
        &self,
        global_context: &DynGlobalClientContext,
        context: &GatewayClientContext,
        common: &GatewayReceiveCommon,
    ) -> Vec<StateTransition<GatewayReceiveStateMachine>> {
        // let contract_id = self.contract_id;
        let funded_common = common.clone();
        let success_context = global_context.clone();
        let txid = self.txid.clone();
        let change = self.change.clone();
        let htlc = self.htlc.clone();
        let gateway_context = context.clone();
        // let gateway = self.gateway.clone();
        vec![StateTransition::new(
            Self::await_preimage_decryption(
                success_context.clone(),
                gateway_context.clone(),
                htlc.clone(),
            ),
            move |dbtx, result, old_state| {
                let htlc = htlc.clone();
                let gateway_context = gateway_context.clone();
                let success_context = success_context.clone();
                Box::pin(Self::transition_incoming_contract_funded(
                    result,
                    old_state,
                    funded_common.clone(),
                    txid,
                    change,
                    htlc,
                    dbtx,
                    success_context,
                    gateway_context,
                ))
            },
        )]
    }

    /// await preimage decryption,
    async fn await_preimage_decryption(
        global_context: DynGlobalClientContext,
        context: GatewayClientContext,
        htlc: Htlc,
    ) -> Result<(), ReceiveError> {
        // TODO: Get rid of polling
        // TODO: distinguish between
        let preimage = loop {
            let contract_id = htlc.payment_hash.into();
            let contract = global_context
                .module_api()
                .get_incoming_contract(contract_id)
                .await;

            if let Ok(contract) = contract {
                match contract.contract.decrypted_preimage {
                    DecryptedPreimage::Pending => {}
                    DecryptedPreimage::Some(preimage) => break preimage,
                    DecryptedPreimage::Invalid => {
                        return Err(ReceiveError::InvalidPreimage);
                    }
                }
            }

            sleep(Duration::from_secs(1)).await;
        };

        // TODO: should we check the preimage validity ourselves here?
        let response = RouteHtlcRequest {
            action: Some(route_htlc_request::Action::CompleteRequest(
                CompleteHtlcsRequest {
                    action: Some(Action::Settle(Settle {
                        preimage: preimage.0.to_vec(),
                    })),
                    incoming_chan_id: htlc.incoming_chan_id,
                    htlc_id: htlc.htlc_id,
                },
            )),
        };

        context
            .lnrpc
            .route_htlc(response)
            .await
            .map_err(|e| ReceiveError::RouteHtlcError)?;

        Ok(())
    }

    async fn refund_incoming_contract(
        dbtx: &mut ClientSMDatabaseTransaction<'_, '_>,
        global_context: DynGlobalClientContext,
        context: GatewayClientContext,
        htlc: Htlc,
        old_state: GatewayReceiveStateMachine,
    ) -> GatewayReceiveStateMachine {
        info!("calling refund");
        let contract_id = htlc.payment_hash.into();
        let contract: IncomingContractAccount = global_context
            .module_api()
            .get_incoming_contract(contract_id)
            .await
            .unwrap(); // FIXME

        let claim_input = contract.claim();
        let client_input = ClientInput::<LightningInput, GatewayClientStateMachines> {
            input: claim_input,
            state_machines: Arc::new(|_, _| vec![]),
            keys: vec![context.redeem_key],
        };

        let (refund_txid, _) = global_context.claim_input(dbtx, client_input).await;

        GatewayReceiveStateMachine {
            common: old_state.common,
            state: GatewayReceiveStates::Refund(RefundState { htlc, refund_txid }),
        }
    }

    async fn transition_incoming_contract_funded(
        result: Result<(), ReceiveError>,
        old_state: GatewayReceiveStateMachine,
        common: GatewayReceiveCommon,
        txid: TransactionId,
        change: Option<OutPoint>,
        htlc: Htlc,
        dbtx: &mut ClientSMDatabaseTransaction<'_, '_>,
        global_context: DynGlobalClientContext,
        context: GatewayClientContext,
    ) -> GatewayReceiveStateMachine {
        assert!(matches!(old_state.state, GatewayReceiveStates::Funding(_)));

        match result {
            Ok(()) => {
                // Success case: funding transaction is accepted
                GatewayReceiveStateMachine {
                    common: old_state.common,
                    state: GatewayReceiveStates::Settled,
                }
            }
            Err(ReceiveError::InvalidPreimage) => {
                Self::refund_incoming_contract(dbtx, global_context, context, htlc, old_state).await
            }
            Err(_) => {
                // Failure case: funding transaction is rejected
                GatewayReceiveStateMachine {
                    common: old_state.common,
                    state: GatewayReceiveStates::Failed, // FIXME: where to send this?
                }
            }
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Decodable, Encodable)]
pub struct AwaitingPreimageDecryption {
    txid: TransactionId,
    change: Option<OutPoint>,
}

#[derive(Debug, Clone, Eq, PartialEq, Decodable, Encodable)]
pub struct PreimageState {
    preimage: Preimage,
}

#[derive(Debug, Clone, Eq, PartialEq, Decodable, Encodable)]
pub struct RefundState {
    htlc: Htlc,
    refund_txid: TransactionId,
}

#[derive(Debug, Clone, Eq, PartialEq, Decodable, Encodable)]
pub struct RefundSuccessState {
    refund_txid: TransactionId,
}

#[derive(Debug, Clone, Eq, PartialEq, Decodable, Encodable)]
pub struct RefundErrorState {
    error: String,
}

impl RefundState {
    fn transitions(
        &self,
        common: &GatewayReceiveCommon,
        global_context: &DynGlobalClientContext,
    ) -> Vec<StateTransition<GatewayReceiveStateMachine>> {
        vec![StateTransition::new(
            Self::await_refund_success(common.clone(), global_context.clone(), self.refund_txid),
            |_dbtx, result, old_state| Box::pin(Self::transition_refund_success(result, old_state)),
        )]
    }

    async fn await_refund_success(
        common: GatewayReceiveCommon,
        global_context: DynGlobalClientContext,
        refund_txid: TransactionId,
    ) -> Result<(), TxSubmissionError> {
        global_context
            .await_tx_accepted(common.operation_id, refund_txid)
            .await
    }

    async fn transition_refund_success(
        result: Result<(), TxSubmissionError>,
        old_state: GatewayReceiveStateMachine,
    ) -> GatewayReceiveStateMachine {
        let refund_txid = match old_state.state {
            GatewayReceiveStates::Refund(refund) => refund.refund_txid,
            _ => panic!("Invalid state transition"),
        };

        match result {
            Ok(_) => {
                // Refund successful
                GatewayReceiveStateMachine {
                    common: old_state.common,
                    state: GatewayReceiveStates::RefundSuccess(RefundSuccessState { refund_txid }),
                }
            }
            Err(_) => {
                // Refund failed
                // TODO: include e-cash notes for recovery? Although, they are in the log …
                GatewayReceiveStateMachine {
                    common: old_state.common,
                    state: GatewayReceiveStates::RefundError(RefundErrorState {
                        error: format!("Refund transaction {refund_txid} was rejected"),
                    }),
                }
            }
        }
    }
}
