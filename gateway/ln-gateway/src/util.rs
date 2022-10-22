use std::{fs::File, net::SocketAddr, path::Path, path::PathBuf};

use cln_plugin::Error;
use fedimint_server::config::{load_from_file, ClientConfig};
use mint_client::{Client, GatewayClientConfig};
use rand::thread_rng;
use secp256k1::{KeyPair, PublicKey};
use tracing::debug;
use url::Url;

/// Build a new federation client with RocksDb and config at a given path
pub fn build_federation_client(
    node_pub_key: PublicKey,
    bind_addr: SocketAddr,
    work_dir: PathBuf,
) -> Result<Client<GatewayClientConfig>, Error> {
    // Create a gateway client configuration
    let client_cfg_path = work_dir.join("client.json");
    let client_cfg: ClientConfig = load_from_file(&client_cfg_path);

    let mut rng = thread_rng();
    let ctx = secp256k1::Secp256k1::new();
    let kp_fed = KeyPair::new(&ctx, &mut rng);

    let client_cfg = GatewayClientConfig {
        client_config: client_cfg,
        redeem_key: kp_fed,
        timelock_delta: 10,
        node_pub_key,
        api: Url::parse(format!("http://{}", bind_addr).as_str())
            .expect("Could not parse URL to generate GatewayClientConfig API endpoint"),
    };

    // TODO: With support for multiple federations and federation registration through config code
    // the gateway should be able back-up all registered federations in a snapshot or db.
    save_federation_client_cfg(work_dir.clone(), &client_cfg);

    // Create a database
    let db_path = work_dir.join("gateway.db");
    let db = fedimint_rocksdb::RocksDb::open(db_path)
        .expect("Error opening DB")
        .into();

    // Create context
    let ctx = secp256k1::Secp256k1::new();

    Ok(Client::new(client_cfg, db, ctx))
}

/// Persist federation client cfg to [`gateway.json`] file
pub fn save_federation_client_cfg(work_dir: PathBuf, client_cfg: &GatewayClientConfig) {
    let path: PathBuf = work_dir.join("gateway.json");
    if !Path::new(&path).is_file() {
        debug!("Creating new gateway cfg file at {}", path.display());
        let file = File::create(path).expect("Could not create gateway cfg file");
        serde_json::to_writer_pretty(file, &client_cfg).expect("Could not write gateway cfg");
    } else {
        debug!("Gateway cfg file already exists at {}", path.display());
        let file = File::open(path).expect("Could not load gateway cfg file");
        serde_json::to_writer_pretty(file, &client_cfg).expect("Could not write gateway cfg");
    }
}
