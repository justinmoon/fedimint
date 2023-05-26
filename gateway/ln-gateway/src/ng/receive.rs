use std::ops::Add;
use std::sync::Arc;
use std::time::Duration;

use bitcoin_hashes::{sha256, Hash};
use fedimint_client::sm::{ClientSMDatabaseTransaction, OperationId, State, StateTransition};
use fedimint_client::transaction::ClientOutput;
use fedimint_client::DynGlobalClientContext;
use fedimint_core::api::GlobalFederationApi;
use fedimint_core::core::{Decoder, OutputOutcome};
use fedimint_core::encoding::{Decodable, Encodable};
use fedimint_core::task::{sleep, timeout};
use fedimint_core::{Amount, OutPoint, TransactionId};
use fedimint_ln_client::api::LnFederationApi;
use fedimint_ln_common::contracts::incoming::{IncomingContract, IncomingContractOffer};
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
}

#[derive(Debug, Clone, Eq, PartialEq, Decodable, Encodable)]
pub enum GatewayReceiveStates {
    HtlcIntercepted(HtlcIntercepted),
    Funding(AwaitingContractAcceptance),
    Funded(AwaitingPreimageDecryption),
    Preimage(PreimageState),
    Settled,
    Refund,
    // AwaitingPreimageDecryption(AwaitingPreimageDecryption),
    // SettledHtlc,
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
            Self::await_preimage_decryption(success_context, gateway_context, htlc),
            move |_dbtx, result, old_state| {
                Box::pin(Self::transition_incoming_contract_funded(
                    result,
                    old_state,
                    funded_common.clone(),
                    txid,
                    change,
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

    async fn transition_incoming_contract_funded(
        result: Result<(), ReceiveError>,
        old_state: GatewayReceiveStateMachine,
        common: GatewayReceiveCommon,
        txid: TransactionId,
        change: Option<OutPoint>,
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
            // TODO: cover failure case where lightning node couldn't settle the htlc
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
