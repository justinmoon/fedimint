pub mod pay;

use std::sync::Arc;

use anyhow::anyhow;
use bitcoin_hashes::Hash;
use fedimint_client::derivable_secret::DerivableSecret;
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
use fedimint_core::{apply, async_trait_maybe_send};
use fedimint_ln_common::config::LightningClientConfig;
use fedimint_ln_common::{LightningCommonGen, LightningModuleTypes, KIND};
use lightning_invoice::Invoice;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use self::pay::{
    GatewayPayCommon, GatewayPayFetchContract, GatewayPayStateMachine, GatewayPayStates,
};
use crate::lnrpc_client::ILnRpcClient;

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
    async fn gateway_pay_bolt11_invoice(&self, invoice: Invoice) -> anyhow::Result<OperationId>;

    /// Subscribe to update to lightning payment
    async fn gateway_subscribe_ln_pay(
        &self,
        operation_id: OperationId,
    ) -> anyhow::Result<UpdateStreamOrOutcome<'_, GatewayExtPayStates>>;
}

#[apply(async_trait_maybe_send!)]
impl GatewayClientExt for Client {
    /// Pays a LN invoice with our available funds
    async fn gateway_pay_bolt11_invoice(&self, invoice: Invoice) -> anyhow::Result<OperationId> {
        let (gateway, instance) = self.get_first_module::<GatewayClientModule>(&KIND);

        self.db()
            .autocommit(
                |dbtx| {
                    let invoice = invoice.clone();
                    Box::pin(async move {
                        let (operation_id, states) = gateway
                            .gateway_pay_bolt11_invoice(
                                &mut dbtx.with_module_prefix(instance.id),
                                invoice,
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
}

#[derive(Debug, Clone)]
pub struct GatewayClientGen;

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
        _module_root_secret: DerivableSecret,
        _notifier: ModuleNotifier<DynGlobalClientContext, <Self::Module as ClientModule>::States>,
    ) -> anyhow::Result<Self::Module> {
        Ok(GatewayClientModule { cfg })
    }
}

#[derive(Debug, Clone)]
pub struct GatewayClientContext {
    lnrpc: Arc<RwLock<dyn ILnRpcClient>>,
}

impl Context for GatewayClientContext {}

#[derive(Debug)]
pub struct GatewayClientModule {
    cfg: LightningClientConfig,
}

impl ClientModule for GatewayClientModule {
    type Common = LightningModuleTypes;
    type ModuleStateMachineContext = GatewayClientContext;
    type States = GatewayClientStateMachines;

    fn context(&self) -> Self::ModuleStateMachineContext {
        todo!()
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
        invoice: Invoice,
    ) -> anyhow::Result<(OperationId, Vec<GatewayClientStateMachines>)> {
        // FIXME: will this prevent us from attempting to pay this invoice a second time
        // if the first attempt fails?
        let operation_id = OperationId(invoice.payment_hash().into_inner());

        let state_machines = vec![GatewayClientStateMachines::Pay(GatewayPayStateMachine {
            common: GatewayPayCommon {
                redeem_key: todo!(),
            },
            state: GatewayPayStates::FetchContract(GatewayPayFetchContract {
                contract_id: todo!(),
                timelock_delta: 10,
            }),
        })];

        Ok((operation_id, state_machines))
    }
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
