use crate::config::{gen_cert_and_key, Peer as ServerPeer, ServerConfig};
use crate::setup::Guardian;
use std::collections::{BTreeMap, HashMap};

use crate::net::peers::ConnectionConfig;
use crate::{CryptoRng, RngCore};
use fedimint_api::config::GenerateConfig;
use fedimint_api::rand::Rand07Compat;
use fedimint_api::{Amount, PeerId};
use fedimint_core::config::{ClientConfig, Node};
use fedimint_core::modules::ln::config::LightningModuleConfig;
use fedimint_core::modules::mint::config::MintConfig;
use fedimint_wallet::config::WalletConfig;
use rand::rngs::OsRng;
use threshold_crypto::serde_impl::SerdeSecret;
use url::Url;

pub fn configgen(guardians: Vec<Guardian>) -> (Vec<(Guardian, ServerConfig)>, ClientConfig) {
    let amount_tiers = vec![Amount::from_sat(1), Amount::from_sat(10)];

    let mut rng = OsRng::new().unwrap();

    let num_peers = guardians.len() as u16;
    let peers = (0..num_peers).map(PeerId::from).collect::<Vec<_>>();
    let max_evil = hbbft::util::max_faulty(peers.len());
    println!(
        "Generating keys such that up to {} peers may fail/be evil",
        max_evil
    );
    let params = SetupConfigParams {
        guardians: guardians.clone(),
        amount_tiers,
    };

    let (config_map, client_config) = trusted_dealer_gen(&peers, &params, &mut rng);

    let server_configs = guardians
        .into_iter()
        .enumerate()
        .map(|(index, guardian)| {
            let peer_id = PeerId::from(index as u16);
            let server_config = config_map.get(&peer_id).unwrap().clone();
            (guardian, server_config)
        })
        .collect();
    (server_configs, client_config)
}
#[derive(Debug)]
pub struct SetupConfigParams {
    pub guardians: Vec<Guardian>,
    pub amount_tiers: Vec<fedimint_api::Amount>,
}

fn trusted_dealer_gen(
    peers: &[PeerId],
    params: &SetupConfigParams,
    mut rng: impl RngCore + CryptoRng,
) -> (BTreeMap<PeerId, ServerConfig>, ClientConfig) {
    let hbbft_port = 18240;
    let api_port = 18340;
    let netinfo = hbbft::NetworkInfo::generate_map(peers.to_vec(), &mut Rand07Compat(&mut rng))
        .expect("Could not generate HBBFT netinfo");
    let epochinfo = hbbft::NetworkInfo::generate_map(peers.to_vec(), &mut Rand07Compat(&mut rng))
        .expect("Could not generate HBBFT netinfo");

    let nodes: Vec<Node> = params
        .guardians
        .iter()
        .map(|peer| {
            let parts: Vec<&str> = peer.connection_string.split('@').collect();
            let part = parts[1].to_string();
            let parts: Vec<&str> = part.split(':').collect();
            Node {
                name: peer.name.clone(),
                url: Url::parse(parts[0]).unwrap(),
            }
        })
        .collect();

    let tls_keys = peers
        .iter()
        .map(|peer| {
            let (cert, key) = gen_cert_and_key(&format!("peer-{}", peer.to_usize())).unwrap();
            (*peer, (cert, key))
        })
        .collect::<HashMap<_, _>>();

    let cfg_peers = netinfo
        .iter()
        .map(|(&id, _)| {
            let id_u16: u16 = id.into();
            let peer = ServerPeer {
                connection: ConnectionConfig {
                    hbbft_addr: format!(
                        "{}:{}",
                        nodes[id_u16 as usize].url.clone(),
                        hbbft_port + id_u16
                    ),
                    api_addr: {
                        let s = format!(
                            "ws://{}:{}",
                            nodes[id_u16 as usize].url.clone(),
                            api_port + id_u16
                        );
                        Url::parse(&s).expect("Could not parse URL")
                    },
                },
                tls_cert: tls_keys[&id].0.clone(),
            };

            (id, peer)
        })
        .collect::<BTreeMap<_, _>>();

    let (wallet_server_cfg, wallet_client_cfg) =
        WalletConfig::trusted_dealer_gen(peers, &(), &mut rng);
    let (mint_server_cfg, mint_client_cfg) =
        MintConfig::trusted_dealer_gen(peers, params.amount_tiers.as_ref(), &mut rng);
    let (ln_server_cfg, ln_client_cfg) =
        LightningModuleConfig::trusted_dealer_gen(peers, &(), &mut rng);

    let server_config = netinfo
        .iter()
        .map(|(&id, netinf)| {
            let id_u16: u16 = id.into();
            let epoch_keys = epochinfo.get(&id).unwrap();
            let config = ServerConfig {
                federation_name: "test".to_string(),
                identity: id,
                hbbft_bind_addr: format!("{}:{}", nodes[id_u16 as usize].url.clone(), hbbft_port),
                api_bind_addr: format!("{}:{}", nodes[id_u16 as usize].url.clone(), api_port),
                tls_cert: tls_keys[&id].0.clone(),
                tls_key: tls_keys[&id].1.clone(),
                peers: cfg_peers.clone(),
                hbbft_sks: SerdeSecret(netinf.secret_key_share().unwrap().clone()),
                hbbft_pk_set: netinf.public_key_set().clone(),
                epoch_sks: SerdeSecret(epoch_keys.secret_key_share().unwrap().clone()),
                epoch_pk_set: epoch_keys.public_key_set().clone(),
                wallet: wallet_server_cfg[&id].clone(),
                mint: mint_server_cfg[&id].clone(),
                ln: ln_server_cfg[&id].clone(),
            };
            (id, config)
        })
        .collect();

    let client_config = ClientConfig {
        federation_name: "test".to_string(),
        nodes,
        mint: mint_client_cfg,
        wallet: wallet_client_cfg,
        ln: ln_client_cfg,
    };

    (server_config, client_config)
}
