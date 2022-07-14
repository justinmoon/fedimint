use bitcoin::secp256k1;
use minimint_api::encoding::{Decodable, Encodable};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Encodable, Decodable)]
pub struct LightningGateway {
    pub mint_pub_key: secp256k1::XOnlyPublicKey,
    pub node_pub_key: secp256k1::PublicKey,
    pub api: String,
}
