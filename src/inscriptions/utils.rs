use super::*;

pub trait ScriptToAddr {
    fn to_address_str(&self, network: Network) -> Option<String>;
}

impl ScriptToAddr for bellscoin::ScriptBuf {
    fn to_address_str(&self, network: Network) -> Option<String> {
        bellscoin::Address::from_script(self, network.into())
            .map(|s| s.to_string())
            .ok()
    }
}

impl ScriptToAddr for &bellscoin::ScriptBuf {
    fn to_address_str(&self, network: Network) -> Option<String> {
        bellscoin::Address::from_script(self, network.into())
            .map(|s| s.to_string())
            .ok()
    }
}

pub fn load_prevouts_for_block<'a>(
    db: Arc<DB>,
    txs: &'a [Transaction],
) -> HashMap<OutPoint, TxOut> {
    let txids_keys = txs
        .iter()
        .skip(1)
        .flat_map(|x| x.input.iter().map(|x| x.previous_output))
        .unique()
        .collect_vec();

    if txids_keys.is_empty() {
        return HashMap::new();
    }

    let prevouts = db
        .prevouts
        .multi_get(txids_keys.iter())
        .into_iter()
        .zip(txids_keys.clone())
        .map(|(v, k)| v.map(|x| (k, x)))
        .collect::<Option<HashMap<_, _>>>()
        .expect("Some prevouts are missing");

    std::thread::spawn(move || {
        db.prevouts.remove_batch(txids_keys.iter());
    });

    prevouts
}
