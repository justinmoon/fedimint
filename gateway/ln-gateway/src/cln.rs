use fedimint_core::Amount;
use serde::{Deserialize, Deserializer, Serialize};

/// The core-lightning `htlc_accepted` event's `amount` field has a "msat"
/// suffix
fn as_fedimint_amount<'de, D>(amount: D) -> Result<Amount, D::Error>
where
    D: Deserializer<'de>,
{
    let amount = String::deserialize(amount)?;
    Ok(Amount::from_msats(
        amount[0..amount.len() - 4].parse::<u64>().unwrap(),
    ))
}

// TODO: upstream these structs to cln-plugin
// See: https://github.com/ElementsProject/lightning/blob/master/doc/PLUGINS.md#htlc_accepted
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Htlc {
    #[serde(deserialize_with = "as_fedimint_amount")]
    pub amount_msat: Amount,
    // TODO: use these to validate we can actually redeem the HTLC in time
    pub cltv_expiry: u32,
    pub cltv_expiry_relative: u32,
    pub payment_hash: bitcoin_hashes::sha256::Hash,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Onion {
    #[serde(default)]
    pub short_channel_id: Option<String>,
    #[serde(deserialize_with = "as_fedimint_amount")]
    pub forward_msat: Amount,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct HtlcAccepted {
    pub htlc: Htlc,
    pub onion: Onion,
}
