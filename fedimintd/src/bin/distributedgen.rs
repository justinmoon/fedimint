use fedimintd::distributed_gen::DistributedGen;
use stabilitypool::PoolConfigGenerator;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // TODO: do we need to add stability pool here?
    DistributedGen::new()?
        .with_default_modules()
        .with_module(PoolConfigGenerator)
        .run()
        .await
}
