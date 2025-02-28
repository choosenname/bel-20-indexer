use super::*;
use crate::inscriptions::types::{Outpoint, TokenHistory};

pub trait ScriptToAddr {
    fn to_address_str(&self, network: Network) -> Option<String>;
}

impl ScriptToAddr for bellscoin::ScriptBuf {
    fn to_address_str(&self, network: Network) -> Option<String> {
        bellscoin::Address::from_script(self, network)
            .map(|s| s.to_string())
            .ok()
    }
}

impl ScriptToAddr for &bellscoin::ScriptBuf {
    fn to_address_str(&self, network: Network) -> Option<String> {
        bellscoin::Address::from_script(self, network)
            .map(|s| s.to_string())
            .ok()
    }
}

pub fn load_prevouts_for_block(
    db: Arc<DB>,
    token_history: &[types::TokenHistory],
) -> anyhow::Result<HashMap<Outpoint, TokenHistory>> {
    let txids_keys = token_history
        .iter()
        .skip(1)
        .map(|history| history.to_location.outpoint)
        .unique()
        .collect_vec();

    if txids_keys.is_empty() {
        return Ok(HashMap::new());
    }

    let prevouts = db
        .prevouts
        .multi_get(txids_keys.iter())
        .into_iter()
        .zip(txids_keys.clone())
        .map(|(v, k)| v.map(|x| (k, x)))
        .collect::<Option<HashMap<_, _>>>()
        .anyhow_with("Some prevouts are missing")?;

    std::thread::spawn(move || {
        db.prevouts.remove_batch(txids_keys.iter());
    });

    Ok(prevouts)
}
