//! Gateway integration test suite
//!
//! This crate contains integration tests for the gateway API
//! and business logic.
mod fixtures;

use std::sync::Arc;
use std::time::Duration;

use assert_matches::assert_matches;
use fedimint_core::sats;
use fedimint_core::task::sleep;
use fedimint_core::util::NextOrPending;
use fedimint_dummy_client::DummyClientExt;
use fedimint_ln_client::{LightningClientExt, LnPayState};
use fedimint_testing::btc::BitcoinTest;
use fedimint_testing::federation::FederationTest;
use ln_gateway::rpc::rpc_client::GatewayRpcClient;
use ln_gateway::rpc::{BalancePayload, ConnectFedPayload, DepositAddressPayload};

#[tokio::test(flavor = "multi_thread")]
async fn gatewayd_supports_connecting_multiple_federations() {
    let (_, rpc, fed1, fed2, _) = fixtures::fixtures(None).await;

    assert_eq!(rpc.get_info().await.unwrap().federations.len(), 0);

    let connection1 = fed1.connection_code();
    let info = rpc
        .connect_federation(ConnectFedPayload {
            connect: connection1.to_string(),
        })
        .await
        .unwrap();

    assert_eq!(info.federation_id, connection1.id);

    let connection2 = fed2.connection_code();
    let info = rpc
        .connect_federation(ConnectFedPayload {
            connect: connection2.to_string(),
        })
        .await
        .unwrap();
    assert_eq!(info.federation_id, connection2.id);

    // FIXME: it would be nice if GatewayTest had a reference to the gateway so
    // I could assert that these were saved into .clients
}

#[tokio::test(flavor = "multi_thread")]
async fn gatewayd_shows_info_about_all_connected_federations() {
    let (_, rpc, fed1, fed2, _) = fixtures::fixtures(None).await;

    assert_eq!(rpc.get_info().await.unwrap().federations.len(), 0);

    let id1 = fed1.connection_code().id;
    let id2 = fed2.connection_code().id;

    connect_federations(&rpc, &[&fed1, &fed2]).await.unwrap();

    let info = rpc.get_info().await.unwrap();

    assert_eq!(info.federations.len(), 2);
    assert!(info
        .federations
        .iter()
        .any(|info| info.federation_id == id1));
    assert!(info
        .federations
        .iter()
        .any(|info| info.federation_id == id2));
}

#[tokio::test(flavor = "multi_thread")]
async fn gatewayd_shows_balance_for_any_connected_federation() -> anyhow::Result<()> {
    let (_, rpc, fed1, fed2, _) = fixtures::fixtures(None).await;
    let id1 = fed1.connection_code().id;
    connect_federations(&rpc, &[&fed1, &fed2]).await.unwrap();

    assert_eq!(
        rpc.get_balance(BalancePayload { federation_id: id1 })
            .await
            .unwrap(),
        sats(0)
    );

    Ok(())
}

/// FIXME: assumes that the gateway's balance is 0 when funciton is called ...
async fn pegin_gateway(
    gateway_rpc: &GatewayRpcClient,
    fed: &FederationTest,
    bitcoin: &Arc<dyn BitcoinTest>,
) -> anyhow::Result<()> {
    let federation_id = fed.connection_code().id;
    let address = gateway_rpc
        .get_deposit_address(DepositAddressPayload { federation_id })
        .await?;
    let amount_sent = bitcoin::Amount::from_sat(100_000);
    bitcoin.send_and_mine_block(&address, amount_sent).await;
    bitcoin.mine_blocks(11).await;

    // Loop for 60 seconds checking to see if the balance updated as expect,
    // and fail the test after 60 seconds without balance update
    let mut count = 0;
    loop {
        if gateway_rpc
            .get_balance(BalancePayload { federation_id })
            .await
            .unwrap()
            == sats(100_000)
        {
            return Ok(());
        }
        count += 1;
        assert!(count < 60, "deposit didn't complete in time");
        sleep(Duration::from_secs(1)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn gatewayd_allows_deposit_to_any_connected_federation() -> anyhow::Result<()> {
    let (_, gateway_rpc, fed, _, bitcoin) = fixtures::fixtures(None).await;
    connect_federations(&gateway_rpc, &[&fed]).await.unwrap();
    pegin_gateway(&gateway_rpc, &fed, &bitcoin).await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn gatewayd_allows_withdrawal_from_any_connected_federation() -> anyhow::Result<()> {
    // todo: implement test case

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn gatewayd_supports_backup_of_any_connected_federation() -> anyhow::Result<()> {
    // todo: implement test case

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn gatewayd_supports_restore_of_any_connected_federation() -> anyhow::Result<()> {
    // todo: implement test case

    Ok(())
}

// Internal payments within a federation should not involve the gateway. See
// Issue #613: Federation facilitates internal payments w/o involving gateway
#[tokio::test(flavor = "multi_thread")]
async fn gatewayd_pays_internal_invoice_within_a_connected_federation() -> anyhow::Result<()> {
    // todo: implement test case

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn gatewayd_pays_outgoing_invoice_to_generic_ln() -> anyhow::Result<()> {
    let fixtures = fixtures::base_fixtures().await;
    let fed = fixtures.new_fed().await;
    let user_client = fed.new_client().await;
    let _gateway = fixtures.new_connected_gateway(&fed).await;
    let (lightning, _lnrpc) = fixtures.lightning();

    // Give user some funds
    let (print_op, outpoint) = user_client.print_money(sats(100_000)).await?;
    user_client
        .await_primary_module_output(print_op, outpoint)
        .await?;

    // Create invoice and have user pay it
    let invoice = lightning
        .invoice(fedimint_core::Amount::from_sats(10_000), None)
        .await?;
    let (pay_op, _contract_id) = user_client.pay_bolt11_invoice(invoice.clone()).await?;
    let mut pay_sub = user_client.subscribe_ln_pay(pay_op).await?.into_stream();
    assert_eq!(pay_sub.ok().await?, LnPayState::Created);
    assert_eq!(pay_sub.ok().await?, LnPayState::Funded);
    assert_matches!(pay_sub.ok().await?, LnPayState::Success { .. });

    // TODO: assert that the returned preimage is correct (it's hard-coded to zeros
    // right now)

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn gatewayd_pays_outgoing_invoice_between_federations_connected() -> anyhow::Result<()> {
    // todo: implement test case

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn gatewayd_intercepts_htlc_and_settles_to_connected_federation() -> anyhow::Result<()> {
    let fixtures = fixtures::base_fixtures().await;
    let fed = fixtures.new_fed().await;
    let user_client = fed.new_client().await;
    let gateway = fixtures.new_connected_gateway(&fed).await;
    let gateway_rpc = gateway.get_rpc().await;
    let bitcoin = fixtures.bitcoin();
    let (lightning, _lnrpc) = fixtures.lightning();

    pegin_gateway(&gateway_rpc, &fed, &bitcoin).await?;

    let invoice_amount = sats(100);
    let (invoice_op, invoice) = user_client
        .create_bolt11_invoice(invoice_amount, "description".into(), None)
        .await?;

    lightning.pay_invoice(&invoice).await?;

    let mut invoice_sub = user_client
        .subscribe_ln_pay(invoice_op)
        .await?
        .into_stream();
    assert_eq!(invoice_sub.ok().await?, LnPayState::Created);
    assert_eq!(invoice_sub.ok().await?, LnPayState::Funded);
    assert_matches!(invoice_sub.ok().await?, LnPayState::Success { .. });
    assert_eq!(invoice_amount, user_client.get_balance().await);

    Ok(())
}

pub async fn connect_federations(
    rpc: &GatewayRpcClient,
    feds: &[&FederationTest],
) -> anyhow::Result<()> {
    for fed in feds {
        let connect = fed.connection_code().to_string();
        rpc.connect_federation(ConnectFedPayload {
            connect: connect.clone(),
        })
        .await?;
    }
    Ok(())
}
