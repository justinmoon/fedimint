use assert_matches::assert_matches;
use fedimint_client::module::gen::ClientModuleGenRegistry;
use fedimint_core::util::NextOrPending;
use fedimint_core::{sats, Amount};
use fedimint_dummy_client::{DummyClientExt, DummyClientGen};
use fedimint_dummy_common::config::DummyGenParams;
use fedimint_dummy_server::DummyGen;
use fedimint_ln_client::{LightningClientExt, LightningClientGen, LnPayState};
use fedimint_ln_common::config::LightningGenParams;
use fedimint_ln_server::LightningGen;
use fedimint_mint_client::MintClientGen;
use fedimint_mint_common::config::MintGenParams;
use fedimint_mint_server::MintGen;
use fedimint_testing::fixtures::Fixtures;
use fedimint_wallet_client::WalletClientGen;
use ln_gateway::ng::receive::Htlc;
use ln_gateway::ng::{
    GatewayClientExt, GatewayClientGen, GatewayExtPayStates, GatewayExtReceiveStates,
};

fn fixtures() -> Fixtures {
    // TODO: Remove dependency on mint (legacy gw client)
    let fixtures = Fixtures::new_primary(1, MintClientGen, MintGen, MintGenParams::default());
    let ln_params = LightningGenParams::regtest(fixtures.bitcoin_rpc());
    fixtures
        .with_module(3, DummyClientGen, DummyGen, DummyGenParams::default())
        .with_module(0, LightningClientGen, LightningGen, ln_params)
}

#[tokio::test(flavor = "multi_thread")]
async fn test_gateway_client_pay_valid_invoice() -> anyhow::Result<()> {
    let fixtures = fixtures();
    let fed = fixtures.new_fed().await;
    let user_client = fed.new_client().await;

    let mut registry = ClientModuleGenRegistry::new();
    registry.attach(MintClientGen);
    registry.attach(GatewayClientGen {
        lightning_client: fixtures.lightning().1,
    });
    let gateway = fed.new_gateway_client(registry).await;
    gateway.register_with_federation().await?;

    // Print money for client2
    let (print_op, outpoint) = user_client.print_money(sats(1000)).await?;
    user_client
        .await_primary_module_output(print_op, outpoint)
        .await?;

    // Create test invoice
    let invoice = fixtures.lightning().0.invoice(sats(250), None).await?;

    // User client pays test invoice
    let (pay_op, contract_id) = user_client.pay_bolt11_invoice(invoice.clone()).await?;
    let mut pay_sub = user_client.subscribe_ln_pay(pay_op).await?.into_stream();
    assert_eq!(pay_sub.ok().await?, LnPayState::Created);
    let funded = pay_sub.ok().await?;
    assert_matches!(funded, LnPayState::Funded);

    let gw_pay_op = gateway.gateway_pay_bolt11_invoice(contract_id).await?;
    let mut gw_pay_sub = gateway
        .gateway_subscribe_ln_pay(gw_pay_op)
        .await?
        .into_stream();
    assert_eq!(gw_pay_sub.ok().await?, GatewayExtPayStates::Created);
    assert_eq!(gw_pay_sub.ok().await?, GatewayExtPayStates::Preimage);
    assert_eq!(gw_pay_sub.ok().await?, GatewayExtPayStates::Success);

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_gateway_client_pay_invalid_invoice() -> anyhow::Result<()> {
    let fixtures = fixtures();
    let fed = fixtures.new_fed().await;
    let user_client = fed.new_client().await;

    let mut registry = ClientModuleGenRegistry::new();
    registry.attach(MintClientGen);
    registry.attach(GatewayClientGen {
        lightning_client: fixtures.lightning().1,
    });
    let gateway = fed.new_gateway_client(registry).await;
    gateway.register_with_federation().await?;

    // Print money for client2
    let (print_op, outpoint) = user_client.print_money(sats(1000)).await?;
    user_client
        .await_primary_module_output(print_op, outpoint)
        .await?;

    // Create test invoice
    let invoice = fixtures
        .lightning()
        .0
        .invalid_invoice(sats(250), None)
        .await?;

    // User client pays test invoice
    let (pay_op, contract_id) = user_client.pay_bolt11_invoice(invoice.clone()).await?;
    let mut pay_sub = user_client.subscribe_ln_pay(pay_op).await?.into_stream();
    assert_eq!(pay_sub.ok().await?, LnPayState::Created);
    let funded = pay_sub.ok().await?;
    assert_matches!(funded, LnPayState::Funded);

    let gw_pay_op = gateway.gateway_pay_bolt11_invoice(contract_id).await?;
    let mut gw_pay_sub = gateway
        .gateway_subscribe_ln_pay(gw_pay_op)
        .await?
        .into_stream();
    assert_eq!(gw_pay_sub.ok().await?, GatewayExtPayStates::Created);
    tracing::info!("created");
    assert_eq!(gw_pay_sub.ok().await?, GatewayExtPayStates::Canceled);
    tracing::info!("canceled");

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_gateway_client_intercept_valid_htlc() -> anyhow::Result<()> {
    let fixtures = fixtures();
    let fed = fixtures.new_fed().await;
    let user_client = fed.new_client().await;

    let mut registry = ClientModuleGenRegistry::new();
    registry.attach(MintClientGen);
    registry.attach(WalletClientGen);
    registry.attach(GatewayClientGen {
        lightning_client: fixtures.lightning().1,
    });
    registry.attach(DummyClientGen);
    let gateway = fed.new_gateway_client(registry).await;
    gateway.register_with_federation().await?;

    // Print money for gateway client
    let (print_op, outpoint) = gateway.print_money(sats(1000)).await?;
    gateway
        .await_primary_module_output(print_op, outpoint)
        .await?;

    // User client creates invoice in federation
    let (_invoice_op, invoice) = user_client
        .create_bolt11_invoice(sats(100), "description".into(), None)
        .await?;

    // Create fake HTLC and run gateway state machine
    let htlc = Htlc {
        payment_hash: *invoice.payment_hash(),
        incoming_amount_msat: Amount::from_msats(invoice.amount_milli_satoshis().unwrap()),
        outgoing_amount_msat: Amount::from_msats(invoice.amount_milli_satoshis().unwrap()),
        incoming_expiry: u32::MAX,
        short_channel_id: 1,
        incoming_chan_id: 1,
        htlc_id: 1,
    };
    let intercept_op = gateway.gateway_intercept_htlc(htlc).await?;
    let mut intercept_sub = gateway
        .gateway_subscribe_ln_receive(intercept_op)
        .await?
        .into_stream();
    assert_eq!(intercept_sub.ok().await?, GatewayExtReceiveStates::Created);
    assert_eq!(intercept_sub.ok().await?, GatewayExtReceiveStates::Funding);

    Ok(())
}
