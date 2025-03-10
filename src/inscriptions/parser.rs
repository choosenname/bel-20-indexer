use super::*;
use crate::inscriptions::types::{HistoryLocation, Outpoint, TokenHistory, TokenHistoryData};

pub struct InitialIndexer {}

pub struct TxidN(pub [u8; 32]);

impl From<TxidN> for Txid {
    fn from(value: TxidN) -> Self {
        Txid::from_slice(&value.0).expect("Unexpected txid")
    }
}

impl From<Outpoint> for OutPoint {
    fn from(value: Outpoint) -> Self {
        Self {
            txid: TxidN(value.txid).into(),
            vout: value.vout,
        }
    }
}

impl From<HistoryLocation> for Location {
    fn from(value: HistoryLocation) -> Self {
        Location {
            outpoint: value.outpoint.into(),
            offset: value.offset,
        }
    }
}

impl InitialIndexer {
    fn parse_block(height: u32, created: u32, ths: &[TokenHistory], token_cache: &mut TokenCache) {
        let mut inscription_idx = 0;
        for th in ths {
            let location = th.to_location.into();
            let owner = th.to.compute_script_hash();
            let txid = TxidN(th.from_location.outpoint.txid).into();
            let vout = th.from_location.outpoint.vout;

            match th.token {
                inscriptions::types::ParsedTokenAction::Deploy {
                    tick,
                    max,
                    lim,
                    dec,
                } => {
                    token_cache.token_actions.push(TokenAction::Deploy {
                        genesis: InscriptionId {
                            txid,
                            index: inscription_idx,
                        },
                        proto: DeployProtoDB {
                            tick,
                            max,
                            lim,
                            dec,
                            supply: Fixed128::ZERO,
                            transfer_count: 0,
                            mint_count: 0,
                            height,
                            created,
                            deployer: th.from.compute_script_hash(),
                            transactions: 1,
                        },
                        owner,
                    });
                    inscription_idx += 1;
                }
                inscriptions::types::ParsedTokenAction::Mint { tick, amt } => {
                    token_cache.token_actions.push(TokenAction::Mint {
                        owner,
                        proto: MintProto::Bel20 { tick, amt },
                        txid,
                        vout,
                    })
                }
                inscriptions::types::ParsedTokenAction::DeployTransfer { tick, amt } => {
                    token_cache.token_actions.push(TokenAction::Transfer {
                        location,
                        owner,
                        proto: TransferProto::Bel20 { tick, amt },
                        txid,
                        vout,
                    });
                    token_cache
                        .all_transfers
                        .insert(th.to_location.into(), TransferProtoDB { tick, amt, height });
                }
                inscriptions::types::ParsedTokenAction::SpentTransfer { .. } => {
                    if th.leaked {
                        token_cache.burned_transfer(location, txid, vout);
                    } else {
                        token_cache.trasferred(location, owner, txid, vout);
                    }
                }
            };
        }
    }

    pub async fn handle(
        token_history_data: TokenHistoryData,
        server: Arc<Server>,
        reorg_cache: Option<Arc<parking_lot::Mutex<crate::reorg::ReorgCache>>>,
    ) -> anyhow::Result<()> {
        let block_height = token_history_data.block_info.height;
        let current_hash = token_history_data.block_info.block_hash;
        let mut last_history_id = server.db.last_history_id.get(()).unwrap_or_default();

        if let Some(cache) = reorg_cache.as_ref() {
            cache.lock().new_block(block_height, last_history_id);
        }

        server.db.block_hashes.set(block_height, current_hash);

        if reorg_cache.is_some() {
            debug!("Syncing block: {} ({})", current_hash, block_height);
        }

        let block = token_history_data;
        let created = block.block_info.created;

        match server.addr_tx.send(server::threads::AddressesToLoad {
            height: block_height,
            addresses: block
                .inscriptions
                .iter()
                .flat_map(|x| vec![x.from.clone(), x.to.clone()])
                .collect(),
        }) {
            Ok(_) => {}
            _ => {
                if !server.token.is_cancelled() {
                    panic!("Failed to send addresses to load");
                }
            }
        }

        if block_height < *START_HEIGHT {
            server.db.last_block.set((), block_height);
            return Ok(());
        }

        if block.inscriptions.is_empty() {
            server.db.last_block.set((), block_height);
            return server.new_hash(block_height, current_hash, &[]).await;
        }

        let mut token_cache = TokenCache::default();

        token_cache.valid_transfers.extend(
            server.db.load_transfers(
                block
                    .inscriptions
                    .iter()
                    .filter(|x| {
                        matches!(
                            x.token,
                            inscriptions::types::ParsedTokenAction::SpentTransfer { .. }
                        )
                    })
                    .map(|k| AddressLocation {
                        address: k.to.compute_script_hash(),
                        location: Location {
                            outpoint: k.to_location.outpoint.into(),
                            offset: 0,
                        },
                    })
                    .collect(),
            ),
        );

        Self::parse_block(block_height, created, &block.inscriptions, &mut token_cache);

        token_cache.load_tokens_data(&server.db)?;

        let history = token_cache
            .process_token_actions(reorg_cache.clone(), &server.holders)
            .into_iter()
            .flat_map(|action| {
                last_history_id += 1;
                let mut results: Vec<(AddressTokenId, HistoryValue)> = vec![];
                let token = action.tick();
                let recipient = action.recipient();
                let key = AddressTokenId {
                    address: recipient,
                    token,
                    id: last_history_id,
                };
                let db_action = TokenHistoryDB::from_token_history(action.clone());
                if let TokenHistoryDB::Send {
                    amt, txid, vout, ..
                } = db_action
                {
                    let sender = action
                        .sender()
                        .expect("Should be in here with the Send action");
                    last_history_id += 1;
                    results.extend([
                        (
                            AddressTokenId {
                                address: sender,
                                token,
                                id: last_history_id,
                            },
                            HistoryValue {
                                height: block_height,
                                action: db_action,
                            },
                        ),
                        (
                            key,
                            HistoryValue {
                                height: block_height,
                                action: TokenHistoryDB::Receive {
                                    amt,
                                    sender,
                                    txid,
                                    vout,
                                },
                            },
                        ),
                    ])
                } else {
                    results.push((
                        key,
                        HistoryValue {
                            action: db_action,
                            height: block_height,
                        },
                    ));
                }
                match server.raw_event_sender.send(results.clone()) {
                    Ok(_) => {}
                    _ => {
                        if !server.token.is_cancelled() {
                            panic!("Failed to send raw event");
                        }
                    }
                }
                results
            })
            .collect_vec();

        if let Some(reorg_cache) = reorg_cache.as_ref() {
            let mut cache = reorg_cache.lock();
            history
                .iter()
                .for_each(|(k, _)| cache.added_history(k.clone()));
        };

        {
            let new_keys = history
                .iter()
                .map(|x| x.0.clone())
                .sorted_unstable_by_key(|x| x.id)
                .collect_vec();
            server.db.block_events.set(block_height, new_keys);

            let keys = history.iter().map(|x| (x.1.action.outpoint(), x.0.clone()));
            server.db.outpoint_to_event.extend(keys)
        }

        server
            .new_hash(block_height, current_hash, &history)
            .await?;

        server.db.address_token_to_history.extend(history);

        token_cache.write_token_data(server.db.clone()).await?;
        token_cache.write_valid_transfers(&server.db)?;

        server.db.last_block.set((), block_height);
        server.db.last_history_id.set((), last_history_id);
        Ok(())
    }
}
