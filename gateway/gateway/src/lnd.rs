use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use anyhow::anyhow;
use async_trait::async_trait;
use bitcoin_hashes::{sha256, Hash};
use fedimint_core::task::TaskGroup;
use secp256k1::PublicKey;
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio_stream::wrappers::ReceiverStream;
use tonic_lnd::lnrpc::{GetInfoRequest, SendRequest};
use tonic_lnd::routerrpc::ForwardHtlcInterceptResponse;
use tonic_lnd::{connect, LndClient};
use tracing::{error, info};

use crate::GatewayError;
// use tracing::instrument;
use crate::{
    gatewaylnrpc::{
        self, CompleteHtlcsRequest, CompleteHtlcsResponse, GetPubKeyResponse,
        GetRouteHintsResponse, PayInvoiceRequest, PayInvoiceResponse,
        SubscribeInterceptHtlcsRequest, SubscribeInterceptHtlcsResponse,
    },
    lnrpc_client::{HtlcStream, ILnRpcClient},
};

type HtlcOutcomeSender = oneshot::Sender<ForwardHtlcInterceptResponse>;

pub struct GatewayLndClient {
    client: LndClient,
    outcomes: Arc<Mutex<HashMap<sha256::Hash, HtlcOutcomeSender>>>,
    task_group: TaskGroup,
}

impl GatewayLndClient {
    pub async fn new(
        host: String,
        port: u32,
        tls_cert: String,
        macaroon: String,
        task_group: TaskGroup,
    ) -> crate::Result<Self> {
        let client = connect(host, port, macaroon, tls_cert).await.map_err(|e| {
            error!("Failed to connect to lnrpc server: {:?}", e);
            GatewayError::Other(anyhow!("Failed to connect to lnrpc server"))
        })?;

        Ok(Self {
            client,
            outcomes: Arc::new(Mutex::new(HashMap::new())),
            task_group,
        })
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
        let mut client = self.client.clone();

        let info = client
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
        let mut client = self.client.clone();

        let send_response = client
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
        subscription: SubscribeInterceptHtlcsRequest,
    ) -> crate::Result<HtlcStream<'a>> {
        let mut client = self.client.clone();
        let channel_size = 1024;

        // Channel to send intercepted htlc to gatewayd for processing
        let (gwd_tx, gwd_rx) =
            mpsc::channel::<Result<SubscribeInterceptHtlcsResponse, tonic::Status>>(channel_size);

        // Channel to send responses to LND after processing intercepted HTLC
        let (lnd_tx, lnd_rx) = mpsc::channel::<ForwardHtlcInterceptResponse>(channel_size);

        let mut htlc_stream = client
            .router()
            .htlc_interceptor(ReceiverStream::new(lnd_rx))
            .await
            .map_err(|e| {
                error!("Failed to connect to lnrpc server: {:?}", e);
                GatewayError::Other(anyhow!("Failed to subscribe to LND htlc stream"))
            })?
            .into_inner();

        let scid = subscription.short_channel_id.clone();

        while let Some(htlc) = match htlc_stream.message().await {
            Ok(htlc) => htlc,
            Err(e) => {
                error!("Error received over HTLC subscriprion: {:?}", e);
                gwd_tx
                    .send(Err(tonic::Status::new(
                        tonic::Code::Internal,
                        e.to_string(),
                    )))
                    .await
                    .unwrap();
                None
            }
        } {
            let response: Option<ForwardHtlcInterceptResponse> = if htlc.outgoing_requested_chan_id
                != scid
            {
                // Pass through: This HTLC doesn't belong to the current subscription
                // Forward it to the next interceptor or next node
                Some(ForwardHtlcInterceptResponse {
                    incoming_circuit_key: htlc.incoming_circuit_key,
                    action: 2,
                    preimage: vec![],
                    failure_message: vec![],
                    failure_code: 0,
                })
            } else {
                // This has a chance of collision since payment_hashes are not guaranteed to be
                // unique TODO: generate unique id for each intercepted HTLC
                let intercepted_htlc_id = sha256::Hash::hash(&htlc.onion_blob);

                // Intercept: This HTLC belongs to the current subscription
                // Send it to the gatewayd for processing
                let intercept = SubscribeInterceptHtlcsResponse {
                    payment_hash: htlc.payment_hash,
                    incoming_amount_msat: htlc.incoming_amount_msat,
                    outgoing_amount_msat: htlc.outgoing_amount_msat,
                    incoming_expiry: htlc.incoming_expiry,
                    short_channel_id: scid,
                    intercepted_htlc_id: intercepted_htlc_id.into_inner().to_vec(),
                };

                match gwd_tx.send(Ok(intercept)).await {
                    Ok(_) => {
                        // Open a channel to receive the outcome of the HTLC processing
                        let (sender, receiver) = oneshot::channel::<ForwardHtlcInterceptResponse>();
                        self.outcomes
                            .lock()
                            .await
                            .insert(intercepted_htlc_id, sender);

                        // If the gateway does not respond within the HTLC expiry,
                        // Automatically respond with a failure message.
                        return tokio::time::timeout(Duration::from_secs(30), async {
                            receiver.await.unwrap_or_else(|e| {
                                error!("Failed to receive outcome of intercepted htlc: {:?}", e);
                                Some(cancel_intercepted_htlc(htlc.incoming_circuit_key))
                            })
                        })
                        .await
                        .unwrap_or_else(|e| {
                            error!("await_htlc_processing error {:?}", e);
                            Some(cancel_intercepted_htlc(htlc.incoming_circuit_key))
                        });
                    }
                    Err(e) => {
                        error!("Failed to send HTLC to gatewayd for processing: {:?}", e);
                        // Request LND to cancel the HTLC
                        Some(cancel_intercepted_htlc(htlc.incoming_circuit_key))
                    }
                }
            };

            if response.is_some() {
                match lnd_tx.send(response.unwrap()).await {
                    Ok(_) => {
                        continue;
                    }
                    Err(e) => {
                        error!("Failed to send response to LND: {:?}", e);
                        // Further action is not necessary.
                        // The HTLC will timeout and LND will automatically cancel it.
                        continue;
                    }
                }
            }
        }

        Ok(Box::pin(ReceiverStream::new(gwd_rx)))
    }

    async fn complete_htlc(
        &self,
        _outcome: CompleteHtlcsRequest,
    ) -> crate::Result<CompleteHtlcsResponse> {
        todo!()
    }
}

fn cancel_intercepted_htlc(key: Option) -> ForwardHtlcInterceptResponse {
    // TODO: Specify a failure code and message
    ForwardHtlcInterceptResponse {
        incoming_circuit_key: key,
        action: 1,
        preimage: vec![],
        failure_message: vec![],
        failure_code: 0,
    }
}
