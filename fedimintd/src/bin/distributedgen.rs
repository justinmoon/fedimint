use std::fs;
use std::path::PathBuf;

use clap::{Parser, Subcommand};
use fedimint_api::task::TaskGroup;
use fedimint_api::Amount;
use fedimintd::distributedgen::{gen_tls, run_dkg};
use fedimintd::encrypt::*;
use tokio_rustls::rustls;
use tracing::info;

#[derive(Parser)]
struct Cli {
    #[clap(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print the latest git commit hash this bin. was build with
    VersionHash,
    /// Creates a connection cert string that must be shared with all other peers
    CreateCert {
        /// Directory to output all the generated config files
        #[arg(long = "out-dir")]
        dir_out_path: PathBuf,

        /// Our external address
        #[arg(long = "address", default_value = "127.0.0.1")]
        address: String,

        /// Our base port, ports may be used from base_port to base_port+10
        #[arg(long = "base-port", default_value = "4000")]
        base_port: u16,

        /// Our node name, must be unique among peers
        #[arg(long = "name")]
        name: String,

        /// The password that encrypts the configs, will prompt if not passed in
        #[arg(long = "password")]
        password: String,
    },
    /// All peers must run distributed key gen at the same time to create configs
    Run {
        /// Directory to output all the generated config files
        #[arg(long = "out-dir")]
        dir_out_path: PathBuf,

        /// Federation name, same for all peers
        #[arg(long = "federation-name", default_value = "Hals_trusty_mint")]
        federation_name: String,

        /// Comma-separated list of connection certs from all peers (including ours)
        #[arg(long = "certs", value_delimiter = ',')]
        certs: Vec<String>,

        /// `bitcoind` json rpc endpoint
        #[arg(long = "bitcoind-rpc", default_value = "127.0.0.1:18443")]
        bitcoind_rpc: String,

        /// Available denominations of notes issues by the federation (comma separated)
        /// default = 1 msat - 1M sats by powers of 10
        #[arg(
        long = "denominations",
        value_delimiter = ',',
        num_args = 1..,
        default_value = "1,10,100,1000,10000,100000,1000000,10000000,100000000,1000000000"
        )]
        denominations: Vec<Amount>,

        /// The password that encrypts the configs, will prompt if not passed in
        #[arg(long = "password")]
        password: String,
    },
}

#[tokio::main]
pub async fn main() {
    let mut task_group = TaskGroup::new();

    let command: Command = Cli::parse().command;
    match command {
        Command::CreateCert {
            dir_out_path,
            address,
            base_port,
            name,
            password,
        } => {
            let salt: [u8; 16] = rand::random();
            fs::write(dir_out_path.join(SALT_FILE), hex::encode(salt)).expect("write error");
            let key = get_key(password, dir_out_path.join(SALT_FILE));
            let config_str = gen_tls(&dir_out_path, address, base_port, name, &key);
            println!("{}", config_str);
        }
        Command::Run {
            dir_out_path,
            federation_name,
            certs,
            bitcoind_rpc,
            denominations,
            password,
        } => {
            let key = get_key(password, dir_out_path.join(SALT_FILE));
            let (pk_bytes, nonce) = encrypted_read(&key, dir_out_path.join(TLS_PK));
            let (server, client) = if let Ok(v) = run_dkg(
                &dir_out_path,
                denominations,
                federation_name,
                certs,
                bitcoind_rpc,
                rustls::PrivateKey(pk_bytes),
                &mut task_group,
            )
            .await
            {
                v
            } else {
                info!("Canceled");
                return;
            };

            let server_path = dir_out_path.join(CONFIG_FILE);
            let config_bytes = serde_json::to_string(&server).unwrap().into_bytes();
            encrypted_write(config_bytes, &key, nonce, server_path);

            let client_path: PathBuf = dir_out_path.join("client.json");
            let client_file = fs::File::create(client_path).expect("Could not create cfg file");
            serde_json::to_writer_pretty(client_file, &client).unwrap();
        }
        Command::VersionHash => {
            println!("{}", env!("GIT_HASH"));
        }
    }
}
