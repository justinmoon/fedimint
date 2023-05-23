use std::sync::Arc;
use std::time::Duration;

use bitcoin_hashes::{sha256, Hash};
use fedimint_client::sm::{ClientSMDatabaseTransaction, OperationId, State, StateTransition};
use fedimint_client::transaction::{ClientInput, ClientOutput, TxSubmissionError};
use fedimint_client::DynGlobalClientContext;
use fedimint_core::encoding::{Decodable, Encodable};
use fedimint_core::task::{sleep, timeout};
use fedimint_core::{Amount, OutPoint, TransactionId};
use fedimint_ln_client::api::LnFederationApi;
use fedimint_ln_common::contracts::incoming::{
    IncomingContract, IncomingContractAccount, IncomingContractOffer,
};
use fedimint_ln_common::contracts::{Contract, DecryptedPreimage, Preimage};
use fedimint_ln_common::{ContractOutput, LightningInput, LightningOutput};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{error, info};

use super::{GatewayClientContext, GatewayClientStateMachines};
use crate::gatewaylnrpc::SubscribeInterceptHtlcsResponse;

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
    HtlcIntercepted(HtlcInterceptedState),
    FundingOffer(FundingOfferState),
    DecryptingPreimage(DecryptingPreimageState),
    // TODO
    // Decrypting(AwaitingContractAcceptanceState),
    Preimage(Preimage), // terminal
    Refunding(RefundingState),
    RefundSuccess(TransactionId), // terminal
    RefundError(String),          // terminal
    FundingFailed(String),        // terminal
    InvalidHtlc(ReceiveError),    // terminal
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
            // FIXME: the arguments shouldn't be so inconsistent
            GatewayReceiveStates::HtlcIntercepted(state) => {
                state.transitions(global_context.clone(), context.clone(), self.common.clone())
            }
            GatewayReceiveStates::FundingOffer(state) => {
                state.transitions(&global_context, &context, &self.common)
            }
            GatewayReceiveStates::DecryptingPreimage(state) => {
                state.transitions(&global_context, &context, &self.common)
            }
            GatewayReceiveStates::Refunding(state) => {
                state.transitions(&self.common, &global_context)
            }
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
pub struct HtlcInterceptedState {
    pub htlc: Htlc,
}

impl HtlcInterceptedState {
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
        let offer: IncomingContractOffer = timeout(
            Duration::from_secs(5),
            global_context.module_api().fetch_offer(htlc.payment_hash),
        )
        .await
        .map_err(|_| ReceiveError::Timeout)?
        .map_err(|_| ReceiveError::FetchContractError)?;
        info!("offer {offer:?}");

        if offer.amount > htlc.outgoing_amount_msat {
            return Err(ReceiveError::ViolatedFeePolicy);
        }
        if offer.hash != htlc.payment_hash {
            return Err(ReceiveError::InvalidOffer);
        }
        Ok(offer)
    }

    // FIXME: don't do this in the transition, do it in a (which???) trigger
    async fn transition_intercept_htlc(
        global_context: DynGlobalClientContext,
        dbtx: &mut ClientSMDatabaseTransaction<'_, '_>,
        result: Result<IncomingContractOffer, ReceiveError>,
        common: GatewayReceiveCommon,
        context: GatewayClientContext,
        htlc: Htlc,
    ) -> GatewayReceiveStateMachine {
        info!("transition_intercept_htlc {result:?}");
        match result {
            Ok(offer) => {
                // Outputs
                info!("transition_intercept_htlc ok ");
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
                // TODO: is this right? Do I need to do anything with this state machine?
                match global_context.fund_output(dbtx, client_output).await {
                    Ok((txid, change)) => {
                        info!("OK");
                        GatewayReceiveStateMachine {
                            common,
                            state: GatewayReceiveStates::FundingOffer(FundingOfferState {
                                txid,
                                change,
                                htlc,
                            }),
                        }
                    }
                    Err(e) => {
                        info!("ERR {e:?}");
                        GatewayReceiveStateMachine {
                            common,
                            state: GatewayReceiveStates::FundingFailed(e.to_string()),
                        }
                    }
                }
            }
            Err(e) => {
                info!("transition_intercept_htlc err {e:?}");
                GatewayReceiveStateMachine {
                    common,
                    state: GatewayReceiveStates::InvalidHtlc(e),
                }
            }
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Decodable, Encodable)]
pub struct FundingOfferState {
    txid: TransactionId,
    change: Option<OutPoint>,
    htlc: Htlc,
}

impl FundingOfferState {
    fn transitions(
        &self,
        global_context: &DynGlobalClientContext,
        context: &GatewayClientContext,
        common: &GatewayReceiveCommon,
    ) -> Vec<StateTransition<GatewayReceiveStateMachine>> {
        let txid = self.txid.clone();
        let common = common.clone();
        let htlc = self.htlc.clone();
        vec![StateTransition::new(
            Self::await_funding_success(common, global_context.clone(), txid),
            move |_dbtx, result, old_state| {
                let htlc = htlc.clone();
                Box::pin(Self::transition_funding_success(result, old_state, htlc))
            },
        )]
    }

    async fn await_funding_success(
        common: GatewayReceiveCommon,
        global_context: DynGlobalClientContext,
        txid: TransactionId,
    ) -> Result<(), TxSubmissionError> {
        global_context
            .await_tx_accepted(common.operation_id, txid)
            .await
    }

    async fn transition_funding_success(
        result: Result<(), TxSubmissionError>,
        old_state: GatewayReceiveStateMachine,
        htlc: Htlc,
    ) -> GatewayReceiveStateMachine {
        let txid = match old_state.state {
            GatewayReceiveStates::FundingOffer(refund) => refund.txid,
            _ => panic!("Invalid state transition"),
        };

        match result {
            Ok(_) => GatewayReceiveStateMachine {
                common: old_state.common,
                state: GatewayReceiveStates::DecryptingPreimage(DecryptingPreimageState {
                    txid: txid,
                    // FIXME: don't hard-code
                    change: None,
                    htlc,
                }),
            },
            Err(e) => GatewayReceiveStateMachine {
                common: old_state.common,
                state: GatewayReceiveStates::FundingFailed(e.to_string()),
            },
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Decodable, Encodable)]
pub struct DecryptingPreimageState {
    txid: TransactionId,
    change: Option<OutPoint>,
    htlc: Htlc,
}

impl DecryptingPreimageState {
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
    ) -> Result<Preimage, ReceiveError> {
        // TODO: Get rid of polling
        // TODO: distinguish between
        let preimage = loop {
            let contract_id = htlc.payment_hash.into();
            let contract = global_context
                .module_api()
                .get_incoming_contract(contract_id)
                .await;

            // TODO: handle error case?
            // if let Ok(contract) = contract {
            match contract {
                Ok(contract) => match contract.contract.decrypted_preimage {
                    DecryptedPreimage::Pending => {}
                    DecryptedPreimage::Some(preimage) => break preimage,
                    DecryptedPreimage::Invalid => {
                        return Err(ReceiveError::InvalidPreimage);
                    }
                },
                Err(e) => {
                    error!("Failed to fetch contract {e:?}");
                }
            }

            sleep(Duration::from_secs(1)).await;
        };

        Ok(preimage)
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
            .expect("we already funded this contract, it should exist"); // FIXME

        let claim_input = contract.claim();
        let client_input = ClientInput::<LightningInput, GatewayClientStateMachines> {
            input: claim_input,
            state_machines: Arc::new(|_, _| vec![]),
            keys: vec![context.redeem_key],
        };

        let (refund_txid, _) = global_context.claim_input(dbtx, client_input).await;

        GatewayReceiveStateMachine {
            common: old_state.common,
            state: GatewayReceiveStates::Refunding(RefundingState { htlc, refund_txid }),
        }
    }

    async fn transition_incoming_contract_funded(
        result: Result<Preimage, ReceiveError>,
        old_state: GatewayReceiveStateMachine,
        common: GatewayReceiveCommon,
        txid: TransactionId,
        change: Option<OutPoint>,
        htlc: Htlc,
        dbtx: &mut ClientSMDatabaseTransaction<'_, '_>,
        global_context: DynGlobalClientContext,
        context: GatewayClientContext,
    ) -> GatewayReceiveStateMachine {
        assert!(matches!(
            old_state.state,
            GatewayReceiveStates::DecryptingPreimage(_)
        ));

        match result {
            Ok(preimage) => {
                // Success case: funding transaction is accepted
                info!("got preimage");
                GatewayReceiveStateMachine {
                    common: old_state.common,
                    state: GatewayReceiveStates::Preimage(preimage),
                }
            }
            Err(ReceiveError::InvalidPreimage) => {
                info!("refunding");
                Self::refund_incoming_contract(dbtx, global_context, context, htlc, old_state).await
            }
            Err(e) => {
                info!("FIXME: this shouldn't happen");
                panic!("this shouldn't happen")
                // // Failure case: funding transaction is rejected
                // GatewayReceiveStateMachine {
                //     common: old_state.common,
                //     state: GatewayReceiveStates::FundingFailed(e),
                // }
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
    // TODO: do we need preimage here?
    preimage: Preimage,
}

#[derive(Debug, Clone, Eq, PartialEq, Decodable, Encodable)]
pub struct RefundingState {
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

#[derive(Debug, Clone, Eq, PartialEq, Decodable, Encodable)]
pub struct CancelHtlcState {
    pub htlc: Htlc,
}

impl RefundingState {
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
            GatewayReceiveStates::Refunding(refund) => refund.refund_txid,
            _ => panic!("Invalid state transition"),
        };

        match result {
            Ok(_) => {
                info!("Refund successful {refund_txid:?}");
                GatewayReceiveStateMachine {
                    common: old_state.common,
                    state: GatewayReceiveStates::RefundSuccess(refund_txid),
                }
            }
            Err(_) => {
                info!("Refund failed {refund_txid:?}");
                GatewayReceiveStateMachine {
                    common: old_state.common,
                    state: GatewayReceiveStates::RefundError(format!(
                        "Refund transaction {refund_txid} was rejected"
                    )),
                }
            }
        }
    }
}
