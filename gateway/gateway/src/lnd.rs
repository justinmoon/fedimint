use std::fmt;

use async_trait::async_trait;
use bitcoin_hashes::{sha256, Hash};
use futures::TryStreamExt;
use secp256k1::PublicKey;
use tokio::sync::Mutex;
// FIXME: is this the right one?
use tokio_stream::StreamExt;
use tonic::Streaming;
use tonic_openssl_lnd::lnrpc::{GetInfoRequest, SendRequest};
use tonic_openssl_lnd::LndClient;
use tracing::{error, info};

use crate::gatewaylnrpc::SubscribeInterceptHtlcsResponse;
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
pub struct GatewayLndClient {
    pub lnd_client: LndClient,
    pub complete_htlc_sender: Mutex<
        Option<
            tokio::sync::mpsc::Sender<tonic_openssl_lnd::routerrpc::ForwardHtlcInterceptResponse>,
        >,
    >,
}

impl GatewayLndClient {
    pub fn new(lnd_client: LndClient) -> Self {
        Self {
            lnd_client,
            complete_htlc_sender: Mutex::new(None),
        }
    }
}

impl fmt::Debug for GatewayLndClient {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "LndClient")
    }
}

#[async_trait]
impl ILnRpcClient for GatewayLndClient {
    async fn pubkey(&self) -> crate::Result<GetPubKeyResponse> {
        let info = self
            .lnd_client
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
            .lnd_client
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
        let (tx, rx) = tokio::sync::mpsc::channel::<
            tonic_openssl_lnd::routerrpc::ForwardHtlcInterceptResponse,
        >(1024);
        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);

        // save sender to state
        {
            let mut state = self.complete_htlc_sender.lock().await;
            *state = Some(tx);
        }

        // TODO: filter out htlcs here?

        // let htlc_stream: Streaming<
        //     std::result::Result<SubscribeInterceptHtlcsResponse, tonic::Status>,
        // > = self
        let htlc_stream = self
            .lnd_client
            .clone()
            .router()
            .htlc_interceptor(stream)
            .await
            .expect("Failed to call subscribe_invoices")
            // .into_inner();
            .into_inner()
            .map(|xx| match xx {
                Ok(x) => Ok(SubscribeInterceptHtlcsResponse {
                    payment_hash: x.payment_hash.clone(),
                    outgoing_amount_msat: x.outgoing_amount_msat,
                    intercepted_htlc_id: sha256::Hash::hash(&x.payment_hash).to_vec(),
                    ..Default::default()
                }),
                // Err(e) => Err(tonic::Status::internal("fixme")),
                Err(e) => {
                    error!("htlc stream error {e:?}");
                    Err(tonic::Status::internal("fixme"))
                }
            });
        // .collect();

        // .and_then(|x| SubscribeInterceptHtlcsResponse {
        //     payment_hash: x.pay,
        // });
        // .map(|x| {
        //     // foo
        //     unimplemented!()
        // });

        Ok(Box::pin(htlc_stream))
    }

    async fn complete_htlc(
        &self,
        _outcome: CompleteHtlcsRequest,
    ) -> crate::Result<CompleteHtlcsResponse> {
        todo!()
    }
}
