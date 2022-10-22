use std::net::SocketAddr;

use axum::{response::IntoResponse, routing::post, Extension, Json, Router};
use axum_macros::debug_handler;
use fedimint_server::modules::ln::contracts::ContractId;
use serde_json::json;
use tokio::sync::mpsc;
use tower_http::cors::CorsLayer;
use tracing::{debug, instrument};

use crate::{
    rpc::GatewayRpcSender, BalancePayload, DepositAddressPayload, DepositPayload, GatewayRequest,
    LnGatewayError, WithdrawPayload,
};

pub async fn run_webserver(
    bind_addr: SocketAddr,
    sender: mpsc::Sender<GatewayRequest>,
) -> axum::response::Result<()> {
    let rpc = GatewayRpcSender::new(sender.clone());

    let app = Router::new()
        .route("/info", post(info))
        .route("/balance", post(balance))
        .route("/address", post(address))
        .route("/deposit", post(deposit))
        .route("/withdraw", post(withdraw))
        .route("/pay_invoice", post(pay_invoice))
        .layer(Extension(rpc))
        .layer(CorsLayer::permissive());

    println!("running webserver \n\n\n\n\n");
    axum::Server::bind(&bind_addr)
        .serve(app.into_make_service())
        .await
        .unwrap();

    Ok(())
}

/// Display gateway ecash token balance
#[debug_handler]
#[instrument(skip_all, err)]
async fn info(_: Extension<GatewayRpcSender>) -> Result<impl IntoResponse, LnGatewayError> {
    // TODO: source actual gateway info
    Ok(Json(
        json!({ "url": "http://127.0.0.1:8080", "federations": "1" }),
    ))
}

/// Display gateway ecash token balance
#[debug_handler]
#[instrument(skip_all, err)]
async fn balance(
    Extension(rpc): Extension<GatewayRpcSender>,
    Json(payload): Json<BalancePayload>,
) -> Result<impl IntoResponse, LnGatewayError> {
    let amount = rpc.send(payload).await?;
    Ok(Json(json!({ "balance_msat": amount.milli_sat })))
}

/// Generate deposit address
#[debug_handler]
#[instrument(skip_all, err)]
async fn address(
    Extension(rpc): Extension<GatewayRpcSender>,
    Json(payload): Json<DepositAddressPayload>,
) -> Result<impl IntoResponse, LnGatewayError> {
    let address = rpc.send(payload).await?;
    Ok(Json(json!({ "address": address })))
}

/// Deposit into a gateway federation.
#[debug_handler]
#[instrument(skip_all, err)]
async fn deposit(
    Extension(rpc): Extension<GatewayRpcSender>,
    Json(payload): Json<DepositPayload>,
) -> Result<impl IntoResponse, LnGatewayError> {
    let txid = rpc.send(payload).await?;
    Ok(Json(json!({ "fedimint_txid": txid.to_string() })))
}

/// Withdraw from a gateway federation.
#[debug_handler]
#[instrument(skip_all, err)]
async fn withdraw(
    Extension(rpc): Extension<GatewayRpcSender>,
    Json(payload): Json<WithdrawPayload>,
) -> Result<impl IntoResponse, LnGatewayError> {
    let txid = rpc.send(payload).await?;
    Ok(Json(json!({ "fedimint_txid": txid.to_string() })))
}

#[instrument(skip_all, err)]
async fn pay_invoice(
    Extension(rpc): Extension<GatewayRpcSender>,
    Json(contract_id): Json<ContractId>,
) -> Result<impl IntoResponse, LnGatewayError> {
    debug!(%contract_id, "Received request to pay invoice");
    rpc.send(contract_id).await?;
    Ok(())
}
