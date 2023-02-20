use std::fmt;

use async_trait::async_trait;
use secp256k1::PublicKey;
use tonic_openssl_lnd::lnrpc::{GetInfoRequest, SendRequest};
use tonic_openssl_lnd::LndClient;
use tracing::info;

use crate::GatewayError;
// use tracing::instrument;
use crate::{
    gatewaylnrpc::{
        self, CompleteHtlcsRequest, CompleteHtlcsResponse, GetPubKeyResponse,
        GetRouteHintsResponse, PayInvoiceRequest, PayInvoiceResponse,
        SubscribeInterceptHtlcsRequest,
    },
    lnrpc_client::{HtlcStream, ILnRpcClient},
};

/// Wrapper that implements Debug
/// (can't impl Debug on LndClient because it has members which don't impl
/// Debug)
pub struct GatewayLndClient(pub LndClient);

impl fmt::Debug for GatewayLndClient {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "LndClient")
    }
}

#[async_trait]
impl ILnRpcClient for GatewayLndClient {
    async fn pubkey(&self) -> crate::Result<GetPubKeyResponse> {
        let info = self
            .0
            .clone()
            .lightning()
            .get_info(GetInfoRequest {})
            .await
            .expect("failed to get info")
            .into_inner();
        let pub_key: PublicKey = info.identity_pubkey.parse().expect("invalid pubkey");
        info!("fetched pubkey {:?}", pub_key);
        Ok(GetPubKeyResponse {
            pub_key: pub_key.serialize().to_vec(),
        })
    }

    async fn routehints(&self) -> crate::Result<GetRouteHintsResponse> {
        // TODO: actually implement this
        Ok(GetRouteHintsResponse {
            route_hints: vec![gatewaylnrpc::get_route_hints_response::RouteHint { hops: vec![] }],
        })
    }

    // FIXME: rename this "invoice" parameter
    async fn pay(&self, invoice: PayInvoiceRequest) -> crate::Result<PayInvoiceResponse> {
        let send_response = self
            .0
            .clone()
            .lightning()
            .send_payment_sync(SendRequest {
                payment_request: invoice.invoice.to_string(),
                ..Default::default()
            })
            .await
            .map_err(|e| anyhow::anyhow!(format!("LND error: {e:?}")))?
            .into_inner();
        info!("send response {:?}", send_response);

        if send_response.payment_preimage.is_empty() {
            return Err(GatewayError::NoPreimage);
        };

        Ok(PayInvoiceResponse {
            preimage: send_response.payment_preimage,
        })
    }

    async fn subscribe_htlcs<'a>(
        &self,
        _subscription: SubscribeInterceptHtlcsRequest,
    ) -> crate::Result<HtlcStream<'a>> {
        Ok(Box::pin(futures::stream::iter(vec![])))
    }

    async fn complete_htlc(
        &self,
        _outcome: CompleteHtlcsRequest,
    ) -> crate::Result<CompleteHtlcsResponse> {
        todo!()
    }
}
