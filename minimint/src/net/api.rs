//! Implements the client API through which users interact with the federation

use crate::config::ServerConfig;
use crate::consensus::MinimintConsensus;
use crate::transaction::Transaction;
use minimint_api::{
    module::{api_endpoint, ApiEndpoint, ApiError},
    FederationModule, TransactionId,
};
use minimint_core::{
    config::{ClientConfig, FeeConsensus},
    modules::{
        ln::config::LightningModuleClientConfig,
        mint::{config::MintClientConfig, Keys},
    },
};
use minimint_core::{gateway::LightningGateway, outcome::TransactionStatus};
use minimint_wallet::config::WalletClientConfig;
use std::fmt::Formatter;
use std::iter::FromIterator;
use std::sync::Arc;
use tracing::debug;

use jsonrpsee::{
    types::{error::CallError, ErrorObject},
    ws_server::WsServerBuilder,
    RpcModule,
};

#[derive(Clone)]
struct State {
    minimint: Arc<MinimintConsensus<rand::rngs::OsRng>>,
}

impl std::fmt::Debug for State {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str("State { ... }")
    }
}

pub async fn run_server(cfg: ServerConfig, minimint: Arc<MinimintConsensus<rand::rngs::OsRng>>) {
    let state = State {
        minimint: minimint.clone(),
    };
    let mut rpc_module = RpcModule::new(state);

    attach_endpoints(&mut rpc_module, server_endpoints(), None);
    attach_endpoints(
        &mut rpc_module,
        minimint.wallet.api_endpoints(),
        Some(minimint.wallet.api_base_name()),
    );
    attach_endpoints(
        &mut rpc_module,
        minimint.mint.api_endpoints(),
        Some(minimint.mint.api_base_name()),
    );
    attach_endpoints(
        &mut rpc_module,
        minimint.ln.api_endpoints(),
        Some(minimint.ln.api_base_name()),
    );

    let server = WsServerBuilder::new()
        .build(&cfg.api_bind_addr)
        .await
        .expect("Could not start API server");

    tracing::info!("running ws server on {}", &cfg.api_bind_addr,);

    server
        .start(rpc_module)
        .expect("Could not start API server")
        .await;
}

fn attach_endpoints<M>(
    rpc_module: &mut RpcModule<State>,
    endpoints: &'static [ApiEndpoint<M>],
    base_name: Option<&str>,
) where
    MinimintConsensus<rand::rngs::OsRng>: AsRef<M>,
    M: Sync,
{
    for endpoint in endpoints {
        let endpoint: &'static ApiEndpoint<M> = endpoint;
        let path = if let Some(base_name) = base_name {
            // This memory leak is fine because it only happens on server startup
            // and path has to live till the end of program anyways.
            Box::leak(format!("/{}{}", base_name, endpoint.path).into_boxed_str())
        } else {
            endpoint.path
        };
        rpc_module
            .register_async_method(path, move |params, state| {
                Box::pin(async move {
                    let params = params.one::<serde_json::Value>()?;
                    (endpoint.handler)((*state.minimint).as_ref(), params)
                        .await
                        .map_err(|e| {
                            jsonrpsee::core::Error::Call(CallError::Custom(ErrorObject::owned(
                                e.code, e.message, None::<()>,
                            )))
                        })
                })
            })
            .expect("Failed to register async method");
    }
}

fn server_endpoints() -> &'static [ApiEndpoint<MinimintConsensus<rand::rngs::OsRng>>] {
    const ENDPOINTS: &[ApiEndpoint<MinimintConsensus<rand::rngs::OsRng>>] = &[
        api_endpoint! {
            "/transaction",
            async |minimint: &MinimintConsensus<rand::rngs::OsRng>, transaction: serde_json::Value| -> TransactionId {
                // deserializing Transaction from json Value always fails
                // we need to convert it to string first
                let string = serde_json::to_string(&transaction).expect("encoding error");
                let transaction: Transaction = serde_json::from_str(&string).map_err(|e| ApiError::bad_request(e.to_string()))?;
                let tx_id = transaction.tx_hash();

                minimint
                    .submit_transaction(transaction)
                    .expect("Could not submit sign request to consensus");

                Ok(tx_id)
            }
        },
        api_endpoint! {
            "/fetch_transaction",
            async |minimint: &MinimintConsensus<rand::rngs::OsRng>, tx_hash: TransactionId| -> TransactionStatus {
                debug!(transaction = %tx_hash, "Recieved request");

                let tx_status = minimint.transaction_status(tx_hash).ok_or_else(|| ApiError::not_found(String::from("transaction not found")))?;

                debug!(transaction = %tx_hash, "Sending outcome");
                Ok(tx_status)
            }
        },
        api_endpoint! {
            "/list_gateways",
            async |minimint: &MinimintConsensus<rand::rngs::OsRng>, _v: ()| -> Vec<LightningGateway> {
                Ok(minimint.list_gateways())
            }
        },
        api_endpoint! {
            "/register_gateway",
            async |minimint: &MinimintConsensus<rand::rngs::OsRng>, gateway: LightningGateway| -> () {
                minimint.register_gateway(gateway);
                Ok(())
            }
        },
        api_endpoint! {
            "/config",
            async |minimint: &MinimintConsensus<rand::rngs::OsRng>, _v: ()| -> ClientConfig {
                let api_endpoints: Vec<String> = minimint
                    .cfg
                    .peers
                    .iter()
                    .map(|(_, peer)| peer.connection.addr.clone())
                    .collect();
                let fee_consensus = FeeConsensus {
                    fee_coin_spend_abs: minimint_api::Amount::ZERO,
                    fee_peg_in_abs: minimint_api::Amount::from_sat(1000),
                    fee_coin_issuance_abs: minimint_api::Amount::ZERO,
                    fee_peg_out_abs: minimint_api::Amount::from_sat(1000),
                    fee_contract_input: minimint_api::Amount::ZERO,
                    fee_contract_output: minimint_api::Amount::ZERO,
                };
                let max_evil = hbbft::util::max_faulty(minimint.cfg.peers.len());
                let mint = MintClientConfig {
                    tbs_pks: Keys::from_iter(minimint.mint.pub_key.clone().into_iter()),
                };
                let wallet = WalletClientConfig {
                    peg_in_descriptor: minimint.cfg.wallet.peg_in_descriptor.clone(),
                    network: minimint.cfg.wallet.network,
                };
                let ln = LightningModuleClientConfig {
                    threshold_pub_key: minimint.cfg.ln.threshold_pub_keys.public_key()

                };
                Ok(ClientConfig { api_endpoints, mint, wallet, ln, fee_consensus, max_evil }) }
        },
    ];

    ENDPOINTS
}
