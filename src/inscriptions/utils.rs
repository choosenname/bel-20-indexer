use super::*;
use nintondo_dogecoin::{Address, ScriptBuf};

pub trait ScriptToAddr {
    fn to_address_str(&self, network: Network) -> Option<String>;
}

impl ScriptToAddr for ScriptBuf {
    fn to_address_str(&self, network: Network) -> Option<String> {
        Address::from_script(self, network)
            .map(|s| s.to_string())
            .ok()
    }
}

impl ScriptToAddr for &ScriptBuf {
    fn to_address_str(&self, network: Network) -> Option<String> {
        Address::from_script(self, network)
            .map(|s| s.to_string())
            .ok()
    }
}

impl ScriptToAddr for String {
    fn to_address_str(&self, network: Network) -> Option<String> {
        let unchecked = nintondo_dogecoin::Address::from_str(self).ok()?;
        if unchecked.is_valid_for_network(*NETWORK) {
            let checked = unchecked.assume_checked().to_string();
            return Some(checked);
        }
        None
    }
}

pub fn load_prevouts_for_block(
    db: Arc<DB>,
    txs: &[Transaction],
) -> anyhow::Result<HashMap<OutPoint, TxOut>> {
    let txids_keys = txs
        .iter()
        .skip(1)
        .flat_map(|x| x.input.iter().map(|x| x.previous_output))
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
