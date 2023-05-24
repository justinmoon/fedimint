use std::time::Duration;

use assert_matches::assert_matches;
use fedimint_client::module::gen::ClientModuleGenRegistry;
use fedimint_client::Client;
use fedimint_core::sats;
use fedimint_core::util::NextOrPending;
use fedimint_dummy_client::{DummyClientExt, DummyClientGen};
use fedimint_dummy_common::config::DummyGenParams;
use fedimint_dummy_server::DummyGen;
use fedimint_ln_client::{LightningClientExt, LightningClientGen, LnPayState, LnReceiveState};
use fedimint_ln_common::config::LightningGenParams;
use fedimint_ln_server::LightningGen;
use fedimint_mint_client::MintClientGen;
use fedimint_mint_common::config::MintGenParams;
use fedimint_mint_server::MintGen;
use fedimint_testing::federation::FederationTest;
use fedimint_testing::fixtures::Fixtures;
use fedimint_testing::gateway::GatewayTest;
use ln_gateway::ng::pay::{GatewayPayFetchContract, GatewayPayStates};
use ln_gateway::ng::{GatewayClientExt, GatewayClientGen};

fn fixtures() -> Fixtures {
    // TODO: Remove dependency on mint (legacy gw client)
    let fixtures = Fixtures::new_primary(1, MintClientGen, MintGen, MintGenParams::default());
    let ln_params = LightningGenParams::regtest(fixtures.bitcoin_rpc());
    fixtures
        .with_module(3, DummyClientGen, DummyGen, DummyGenParams::default())
        .with_module(0, LightningClientGen, LightningGen, ln_params)
}

/// Setup a gateway connected to the fed and client
async fn gateway(fixtures: &Fixtures, fed: &FederationTest, client: &Client) -> GatewayTest {
    let gateway = fixtures.new_connected_gateway(fed).await;
    let node_pub_key = gateway.last_registered().await.node_pub_key;
    client.set_active_gateway(&node_pub_key).await.unwrap();
    gateway
}

#[tokio::test(flavor = "multi_thread")]
async fn test_hello() -> anyhow::Result<()> {
    let fixtures = fixtures();
    let fed = fixtures.new_fed().await;
    let user_client = fed.new_client().await;

    // Create gateway client
    let mut registry = ClientModuleGenRegistry::new();
    registry.attach(MintClientGen);
    registry.attach(GatewayClientGen);
    let gateway = fed.new_gateway_client(registry).await;
    gateway.register_with_federation().await?;

    // Print money for client2
    let (print_op, outpoint) = user_client.print_money(sats(1000)).await?;
    user_client
        .await_primary_module_output(print_op, outpoint)
        .await?;

    // Create test invoice
    let invoice = fixtures.lightning().invoice(sats(250), None).await?;

    // User client pays test invoice
    let pay_op = user_client.pay_bolt11_invoice(invoice.clone()).await?;
    let mut pay_sub = user_client.subscribe_ln_pay(pay_op).await?.into_stream();
    assert_eq!(pay_sub.ok().await?, LnPayState::Created);
    assert_matches!(pay_sub.ok().await?, LnPayState::Funded);

    // Gateway facilitates payment on behalf of user
    let gw_pay_op = gateway.gateway_pay_bolt11_invoice(invoice.clone()).await?;
    // let mut sub2 = gateway.gateway_subscribe_ln_pay(op).await?.into_stream();

    // assert_eq!(
    //     sub2.ok().await?,
    //     GatewayPayStates::FetchContract(GatewayPayFetchContract {
    //         contract_id: (*invoice.payment_hash()).into(),
    //         timelock_delta: 10,
    //     })
    // );

    fedimint_core::task::sleep(Duration::from_secs(30)).await;

    Ok(())
}
