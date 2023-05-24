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
    let (client1, client2) = fed.two_clients().await;

    let mut registry = ClientModuleGenRegistry::new();
    // registry.attach(LightningClientGen);
    registry.attach(MintClientGen);
    registry.attach(GatewayClientGen);

    let gateway = fed.new_gateway_client(registry).await;
    gateway.register_with_federation().await?;

    // Print money for client2
    let (op, outpoint) = client2.print_money(sats(1000)).await?;
    client2.await_primary_module_output(op, outpoint).await?;

    let (op, invoice) = client1
        .create_bolt11_invoice(sats(250), "description".to_string(), None)
        .await?;
    let mut sub1 = client1.subscribe_ln_receive(op).await?.into_stream();
    assert_eq!(sub1.ok().await?, LnReceiveState::Created);
    assert_matches!(sub1.ok().await?, LnReceiveState::WaitingForPayment { .. });

    let op = gateway.gateway_pay_bolt11_invoice(invoice.clone()).await?;
    // let mut sub2 = gateway.gateway_subscribe_ln_pay(op).await?.into_stream();

    // assert_eq!(
    //     sub2.ok().await?,
    //     GatewayPayStates::FetchContract(GatewayPayFetchContract {
    //         contract_id: (*invoice.payment_hash()).into(),
    //         timelock_delta: 10,
    //     })
    // );

    Ok(())
}
