use std::sync::Arc;

use fedimint_dummy_client::DummyClientGen;
use fedimint_dummy_common::config::DummyGenParams;
use fedimint_dummy_server::DummyGen;
use fedimint_ln_client::LightningClientGen;
use fedimint_ln_common::config::LightningGenParams;
use fedimint_ln_server::LightningGen;
use fedimint_logging::TracingSetup;
use fedimint_mint_client::MintClientGen;
use fedimint_mint_common::config::MintGenParams;
use fedimint_mint_server::MintGen;
use fedimint_testing::btc::mock::FakeBitcoinTest;
use fedimint_testing::btc::BitcoinTest;
use fedimint_testing::federation::FederationTest;
use fedimint_testing::fixtures::Fixtures;
use fedimint_testing::gateway::GatewayTest;
use fedimint_wallet_client::config::WalletGenParams;
use fedimint_wallet_client::WalletClientGen;
use fedimint_wallet_server::WalletGen;
use ln_gateway::rpc::rpc_client::GatewayRpcClient;

pub async fn base_fixtures() -> Fixtures {
    // FIXME: gateway on-chain deposits were failing with the dummy module. It would
    // reach the "Confirmed" state but not the "Claimed" state. So I switched to
    // using the ecash module which didn't have this problem.
    let _ = TracingSetup::default().init();
    // let mut fixtures =
    //     Fixtures::new_primary(1, DummyClientGen, DummyGen,
    // DummyGenParams::default()).await;
    let mut fixtures =
        Fixtures::new_primary(1, MintClientGen, MintGen, MintGenParams::default()).await;
    let ln_params = LightningGenParams::regtest(fixtures.bitcoin_rpc());
    fixtures = fixtures.with_module(0, LightningClientGen, LightningGen, ln_params);
    let wallet_params = WalletGenParams::regtest(fixtures.bitcoin_rpc());
    fixtures = fixtures.with_module(2, WalletClientGen, WalletGen, wallet_params);
    fixtures = fixtures.with_module(3, DummyClientGen, DummyGen, DummyGenParams::default());
    fixtures
}

pub async fn fixtures(
    password: Option<String>,
) -> (
    GatewayTest,
    GatewayRpcClient,
    FederationTest,
    FederationTest,
    Arc<dyn BitcoinTest>,
) {
    let fixtures = base_fixtures().await;
    let gateway = fixtures.new_gateway(password).await;
    let client = gateway.get_rpc().await;

    let fed1 = fixtures.new_fed().await;
    let fed2 = fixtures.new_fed().await;

    // TODO: Source this from the Fixtures, based on test environment
    // let bitcoin = Box::new(FakeBitcoinTest::new());
    let bitcoin = fixtures.bitcoin();

    (gateway, client, fed1, fed2, bitcoin)
}
