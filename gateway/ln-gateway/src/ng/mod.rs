pub mod pay;

use std::env;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context as AnyhowContext};
use async_stream::stream;
use bitcoin_hashes::Hash;
use fedimint_client::derivable_secret::{ChildId, DerivableSecret};
use fedimint_client::module::gen::ClientModuleGen;
use fedimint_client::module::ClientModule;
use fedimint_client::sm::util::MapStateTransitions;
use fedimint_client::sm::{Context, DynState, ModuleNotifier, OperationId, State};
use fedimint_client::{
    sm_enum_variant_translation, Client, DynGlobalClientContext, UpdateStreamOrOutcome,
};
use fedimint_core::core::{IntoDynInstance, ModuleInstanceId};
use fedimint_core::db::{AutocommitError, Database, ModuleDatabaseTransaction};
use fedimint_core::encoding::{Decodable, Encodable};
use fedimint_core::module::ExtendsCommonModuleGen;
use fedimint_core::task::TaskGroup;
use fedimint_core::{apply, async_trait_maybe_send};
use fedimint_ln_client::api::LnFederationApi;
use fedimint_ln_client::contracts::ContractId;
use fedimint_ln_common::config::{GatewayFee, LightningClientConfig};
use fedimint_ln_common::route_hints::RouteHint;
use fedimint_ln_common::{LightningCommonGen, LightningGateway, LightningModuleTypes, KIND};
use lightning::routing::gossip::RoutingFees;
use secp256k1::{All, KeyPair, PublicKey, Secp256k1};
use serde::{Deserialize, Serialize};
use tracing::info;
use url::Url;

use self::pay::{GatewayPayCommon, GatewayPayInvoice, GatewayPayStateMachine, GatewayPayStates};
use crate::gatewaylnrpc::GetNodeInfoResponse;
use crate::lnd::GatewayLndClient;
use crate::lnrpc_client::{ILnRpcClient, NetworkLnRpcClient};
use crate::LightningMode;

const GW_ANNOUNCEMENT_TTL: Duration = Duration::from_secs(600);

/// The high-level state of a reissue operation started with
/// [`LightningClientExt::pay_bolt11_invoice`].
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub enum GatewayExtPayStates {
    Created,
    FetchedContract,
    BuyPreimage,
    Preimage,
    Success,
    Fail,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GatewayMeta {
    Pay,
}

#[apply(async_trait_maybe_send!)]
pub trait GatewayClientExt {
    /// Pay lightning invoice on behalf of federation user
    async fn gateway_pay_bolt11_invoice(
        &self,
        contract_id: ContractId,
    ) -> anyhow::Result<OperationId>;

    /// Subscribe to update to lightning payment
    async fn gateway_subscribe_ln_pay(
        &self,
        operation_id: OperationId,
    ) -> anyhow::Result<UpdateStreamOrOutcome<'_, GatewayExtPayStates>>;

    /// Register gateway with federation
    async fn register_with_federation(&self) -> anyhow::Result<()>;
}

#[apply(async_trait_maybe_send!)]
impl GatewayClientExt for Client {
    /// Pays a LN invoice with our available funds
    async fn gateway_pay_bolt11_invoice(
        &self,
        contract_id: ContractId,
    ) -> anyhow::Result<OperationId> {
        info!("gateway_pay_bolt11_invoice()");
        let (gateway, instance) = self.get_first_module::<GatewayClientModule>(&KIND);

        self.db()
            .autocommit(
                |dbtx| {
                    Box::pin(async move {
                        let (operation_id, states) = gateway
                            .gateway_pay_bolt11_invoice(
                                &mut dbtx.with_module_prefix(instance.id),
                                contract_id,
                            )
                            .await?;

                        let dyn_states = states
                            .into_iter()
                            .map(|s| s.into_dyn(instance.id))
                            .collect();

                        self.add_state_machines(dbtx, dyn_states).await?;
                        self.add_operation_log_entry(
                            dbtx,
                            operation_id,
                            KIND.as_str(),
                            GatewayMeta::Pay,
                        )
                        .await;

                        Ok(operation_id)
                    })
                },
                Some(100),
            )
            .await
            .map_err(|e| match e {
                AutocommitError::ClosureError { error, .. } => error,
                AutocommitError::CommitFailed { last_error, .. } => {
                    anyhow!("Commit to DB failed: {last_error}")
                }
            })
    }

    async fn gateway_subscribe_ln_pay(
        &self,
        operation_id: OperationId,
    ) -> anyhow::Result<UpdateStreamOrOutcome<'_, GatewayExtPayStates>> {
        unimplemented!()
    }

    /// Register this gateway with the federation
    async fn register_with_federation(&self) -> anyhow::Result<()> {
        let (gateway, instance) = self.get_first_module::<GatewayClientModule>(&KIND);
        let route_hints = vec![];
        let config = gateway.to_gateway_registration_info(route_hints, GW_ANNOUNCEMENT_TTL);
        instance.api.register_gateway(&config).await?;
        Ok(())
    }
}

async fn create_lightning_client(mode: LightningMode) -> anyhow::Result<Arc<dyn ILnRpcClient>> {
    // FIXME: remove task group requirement from GatewayLndClient
    let task_group = TaskGroup::new();
    let lnrpc: Arc<dyn ILnRpcClient> = match mode {
        LightningMode::Cln { cln_extension_addr } => {
            info!(
                "Gateway configured to connect to remote LnRpcClient at \n cln extension address: {:?} ",
                cln_extension_addr
            );
            Arc::new(NetworkLnRpcClient::new(cln_extension_addr).await?)
        }
        LightningMode::Lnd {
            lnd_rpc_addr,
            lnd_tls_cert,
            lnd_macaroon,
        } => {
            info!(
                "Gateway configured to connect to LND LnRpcClient at \n address: {:?},\n tls cert path: {:?},\n macaroon path: {} ",
                lnd_rpc_addr, lnd_tls_cert, lnd_macaroon
            );
            Arc::new(
                GatewayLndClient::new(lnd_rpc_addr, lnd_tls_cert, lnd_macaroon, task_group).await?,
            )
        }
    };

    Ok(lnrpc)
}

#[derive(Debug, Clone)]
pub struct GatewayClientGen {
    pub lightning_client: Arc<dyn ILnRpcClient>,
}

impl ExtendsCommonModuleGen for GatewayClientGen {
    type Common = LightningCommonGen;
}

#[apply(async_trait_maybe_send!)]
impl ClientModuleGen for GatewayClientGen {
    type Module = GatewayClientModule;
    type Config = LightningClientConfig;

    async fn init(
        &self,
        cfg: Self::Config,
        _db: Database,
        module_root_secret: DerivableSecret,
        notifier: ModuleNotifier<DynGlobalClientContext, <Self::Module as ClientModule>::States>,
    ) -> anyhow::Result<Self::Module> {
        let GetNodeInfoResponse { pub_key, alias: _ } = self.lightning_client.info().await?;
        let node_pub_key =
            PublicKey::from_slice(&pub_key).map_err(|e| anyhow!("Invalid node pubkey {}", e))?;
        info!("pubkey: {node_pub_key:?}");
        let api: Url = env::var("FM_GATEWAY_API_ADDR")
            .context("FM_GATEWAY_API_ADDR not found")?
            .parse()?;
        let fees: GatewayFee = env::var("FM_GATEWAY_FEES")
            .context("FM_GATEWAY_FEES not found")?
            .parse()?;
        Ok(GatewayClientModule {
            cfg,
            notifier,
            redeem_key: module_root_secret
                .child_key(ChildId(0))
                .to_secp_key(&Secp256k1::new()),
            node_pub_key,
            lightning_client: self.lightning_client.clone(),
            timelock_delta: 10, // FIXME: don't hardcode
            api,
            mint_channel_id: 1, // FIXME: don't hardcode
            fees: fees.0,
        })
    }
}

#[derive(Debug, Clone)]
pub struct GatewayClientContext {
    lnrpc: Arc<dyn ILnRpcClient>,
    redeem_key: bitcoin::KeyPair,
    timelock_delta: u64,
}

impl Context for GatewayClientContext {}

#[derive(Debug)]
pub struct GatewayClientModule {
    cfg: LightningClientConfig,
    pub notifier: ModuleNotifier<DynGlobalClientContext, GatewayClientStateMachines>,
    // secp: Secp256k1<All>,
    redeem_key: KeyPair,
    node_pub_key: PublicKey,
    timelock_delta: u64, // FIXME: don't hard-code
    // FIXME: this is used for gateway registration
    // Should this happen inside or outside the client?
    api: Url,
    mint_channel_id: u64,
    fees: RoutingFees,
    lightning_client: Arc<dyn ILnRpcClient>,
}

impl ClientModule for GatewayClientModule {
    type Common = LightningModuleTypes;
    type ModuleStateMachineContext = GatewayClientContext;
    type States = GatewayClientStateMachines;

    fn context(&self) -> Self::ModuleStateMachineContext {
        Self::ModuleStateMachineContext {
            lnrpc: self.lightning_client.clone(),
            redeem_key: self.redeem_key,
            timelock_delta: self.timelock_delta,
        }
    }

    fn input_amount(
        &self,
        _input: &<Self::Common as fedimint_core::module::ModuleCommon>::Input,
    ) -> fedimint_core::module::TransactionItemAmount {
        todo!()
    }

    fn output_amount(
        &self,
        _output: &<Self::Common as fedimint_core::module::ModuleCommon>::Output,
    ) -> fedimint_core::module::TransactionItemAmount {
        todo!()
    }
}

impl GatewayClientModule {
    /// This creates and returns the states which calling client can launch in
    /// the executor
    async fn gateway_pay_bolt11_invoice(
        &self,
        dbtx: &mut ModuleDatabaseTransaction<'_>,
        contract_id: ContractId,
    ) -> anyhow::Result<(OperationId, Vec<GatewayClientStateMachines>)> {
        // FIXME: will this prevent us from attempting to pay this invoice a second time
        // if the first attempt fails?
        let operation_id = OperationId(contract_id.into_inner());

        let state_machines = vec![GatewayClientStateMachines::Pay(GatewayPayStateMachine {
            common: GatewayPayCommon { operation_id },
            state: GatewayPayStates::PayInvoice(GatewayPayInvoice {
                contract_id,
                timelock_delta: 10,
            }),
        })];

        Ok((operation_id, state_machines))
    }
    pub fn to_gateway_registration_info(
        &self,
        route_hints: Vec<RouteHint>,
        time_to_live: Duration,
    ) -> LightningGateway {
        LightningGateway {
            mint_channel_id: self.mint_channel_id,
            mint_pub_key: self.redeem_key.x_only_public_key().0,
            node_pub_key: self.node_pub_key,
            api: self.api.clone(),
            route_hints,
            valid_until: fedimint_core::time::now() + time_to_live,
            fees: self.fees,
        }
    }

    // async fn subscribe_pay(
    //     &self,
    //     operation_id: OperationId,
    // ) -> anyhow::Result<UpdateStreamOrOutcome<'_, GatewayPayStates>> {
    //     let mut stream = self.notifier.subscribe(operation_id).await;
    //     // loop {
    //     //     match stream.next().await {
    //     //         Some(LightningClientStateMachines::Pay(state)) => match
    // state.state {     //
    // LightningPayStates::Refunded(refund_txid) => {     //
    // return Ok(refund_txid);     //             }
    //     //             LightningPayStates::Failure(reason) => {
    //     //                 return Err(LightningPayError::Failed(reason))
    //     //             }
    //     //             _ => {}
    //     //         },
    //     //         Some(_) => {}
    //     //         None =>
    //     //     }
    //     // }
    //     stream! {
    //         while let Some(state) = stream.next().await {
    //             yield state
    //         }
    //     }
    // }

    // async fn await_fetched(
    //     &self,
    //     operation_id: OperationId,
    // ) -> Result<TransactionId, LightningPayError> {
    //     let mut stream = self.notifier.subscribe(operation_id).await;
    //     loop {
    //         match stream.next().await {
    //             Some(LightningClientStateMachines::Pay(state)) => match
    // state.state {                 LightningPayStates::Refunded(refund_txid)
    // => {                     return Ok(refund_txid);
    //                 }
    //                 LightningPayStates::Failure(reason) => {
    //                     return Err(LightningPayError::Failed(reason))
    //                 }
    //                 _ => {}
    //             },
    //             Some(_) => {}
    //             None => {}
    //         }
    //     }
    // }
}

#[derive(Debug, Clone, Eq, PartialEq, Decodable, Encodable)]
pub enum GatewayClientStateMachines {
    Pay(GatewayPayStateMachine),
}

impl IntoDynInstance for GatewayClientStateMachines {
    type DynType = DynState<DynGlobalClientContext>;

    fn into_dyn(self, instance_id: ModuleInstanceId) -> Self::DynType {
        DynState::from_typed(instance_id, self)
    }
}

impl State for GatewayClientStateMachines {
    type ModuleContext = GatewayClientContext;
    type GlobalContext = DynGlobalClientContext;

    fn transitions(
        &self,
        context: &Self::ModuleContext,
        global_context: &Self::GlobalContext,
    ) -> Vec<fedimint_client::sm::StateTransition<Self>> {
        match self {
            GatewayClientStateMachines::Pay(pay_state) => {
                sm_enum_variant_translation!(
                    pay_state.transitions(context, global_context),
                    GatewayClientStateMachines::Pay
                )
            }
        }
    }

    fn operation_id(&self) -> fedimint_client::sm::OperationId {
        match self {
            GatewayClientStateMachines::Pay(pay_state) => pay_state.operation_id(),
        }
    }
}
