use super::*;

generate_db_code! {
    token_to_meta: TokenTick => UsingSerde<TokenMetaDB>,
    address_location_to_transfer: AddressLocation => UsingSerde<TransferProtoDB>,
    address_token_to_balance: AddressToken => UsingSerde<TokenBalance>,
    address_token_to_history: AddressTokenId => UsingSerde<HistoryValue>,
    block_hashes: u64 => UsingConsensus<BlockHash>,
    prevouts: UsingConsensus<OutPoint> => UsingConsensus<TxOut>,
    last_block: () => u64,
    last_history_id: () => u64,
    proof_of_history: u64 => UsingConsensus<sha256::Hash>,
    block_events: u64 => Vec<AddressTokenId>,
    fullhash_to_address: FullHash => String
}

impl DB {
    pub fn load_token_accounts(
        &self,
        keys: HashSet<(FullHash, [u8; 4])>,
    ) -> HashMap<AddressToken, TokenBalance> {
        let db_keys = keys
            .into_iter()
            .map(|x| AddressToken {
                address: x.0,
                token: x.1,
            })
            .collect_vec();

        self.address_token_to_balance
            .multi_get(db_keys.iter().collect_vec())
            .into_iter()
            .zip(db_keys)
            .flat_map(|(v, k)| v.map(|v| (k, v)))
            .collect()
    }

    pub fn load_transfers(
        &self,
        keys: BTreeSet<AddressLocation>,
    ) -> Vec<(Location, (FullHash, TransferProto))> {
        let result = self
            .address_location_to_transfer
            .iter()
            .filter_map(|(k, v)| {
                if keys
                    .get(&AddressLocation {
                        address: k.address.clone(),
                        location: Location {
                            offset: 0,
                            outpoint: k.location.outpoint,
                        },
                    })
                    .is_some()
                {
                    Some((k.location, (k.address, v.into())))
                } else {
                    None
                }
            })
            .collect_vec();

        self.address_location_to_transfer
            .remove_batch(
                result
                    .iter()
                    .map(|(location, (address, _))| AddressLocation {
                        address: *address,
                        location: *location,
                    }),
            );

        result
    }
}
