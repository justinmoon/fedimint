use fedimint_core::Amount;
use fedimint_dummy_common::config::{DummyGenParams, DummyGenParamsConsensus};
use fedimint_dummy_server::DummyGen;
use fedimintd::distributed_gen::DistributedGen;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    DistributedGen::new()?
        .with_default_modules()
        .with_module(DummyGen)
        .with_extra_module_gens_params(
            3,
            fedimint_dummy_common::KIND,
            DummyGenParams {
                local: Default::default(),
                consensus: DummyGenParamsConsensus {
                    tx_fee: Amount::ZERO,
                },
            },
        )
        .run()
        .await
}
