use fedimint_dummy_server::DummyGen;
use fedimintd::distributed_gen::DistributedGen;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    DistributedGen::new()?
        .with_default_modules()
        .with_module(DummyGen)
        .run()
        .await
}
