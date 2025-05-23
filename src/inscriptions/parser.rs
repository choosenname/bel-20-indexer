use super::*;

pub struct ParseInscription<'a> {
    tx: &'a Transaction,
    input_idx: usize,
    inscription_idx: &'a mut u32,
    inputs_cum: &'a [u64],
}

pub struct InitialIndexer {}

impl InitialIndexer {
    fn parse_block(
        height: u32,
        created: u32,
        txs: &[Transaction],
        prevouts: &HashMap<OutPoint, TxOut>,
        token_cache: &mut TokenCache,
    ) {
        let mut transfers = vec![];

        for tx in txs {
            if tx.is_coin_base() {
                continue;
            }
            let mut inscription_idx = 0;
            let txid = tx.txid();

            let inputs_cum = InscriptionSearcher::calc_offsets(tx, prevouts)
                .expect("failed to find all txos to calculate offsets");

            for (idx, txin) in tx.input.iter().enumerate() {
                transfers.extend(
                    token_cache
                        .valid_transfers
                        .range(
                            Location {
                                outpoint: txin.previous_output,
                                offset: 0,
                            }..=Location {
                                outpoint: txin.previous_output,
                                offset: u64::MAX,
                            },
                        )
                        .map(|(k, (address, proto))| (*k, (*address, proto.clone()))),
                );

                for &(location, _) in &transfers {
                    if location.outpoint != txin.previous_output {
                        continue;
                    }

                    if let Ok((vout, _)) = InscriptionSearcher::get_output_index_by_input(
                        inputs_cum.get(idx).map(|&x| x + location.offset),
                        &tx.output,
                    ) {
                        if tx.output[vout as usize].script_pubkey.is_op_return() {
                            token_cache.burned_transfer(location, txid, vout);
                        } else {
                            let owner =
                                tx.output[vout as usize].script_pubkey.compute_script_hash();
                            token_cache.transferred(location, owner, txid, vout);
                        };
                    } else {
                        token_cache.transferred(
                            location,
                            prevouts
                                .get(&txin.previous_output)
                                .unwrap()
                                .script_pubkey
                                .compute_script_hash(),
                            tx.txid(),
                            0,
                        );
                    }
                }

                for inc in Self::parse_inscriptions(ParseInscription {
                    tx,
                    input_idx: idx,
                    inscription_idx: &mut inscription_idx,
                    inputs_cum: &inputs_cum,
                }) {
                    if inc.genesis.index == 0
                        || height as usize >= *MULTIPLE_INPUT_BEL_20_ACTIVATION_HEIGHT
                    {
                        if let Some(proto) = token_cache.parse_token_action(&inc, height, created) {
                            transfers.push((
                                inc.location,
                                (inc.owner, TransferProtoDB::from_proto(proto, height)),
                            ))
                        };
                    }
                }
            }
        }
    }

    pub async fn handle(
        block_height: u32,
        server: Arc<Server>,
        reorg_cache: Option<Arc<parking_lot::Mutex<crate::reorg::ReorgCache>>>,
    ) -> anyhow::Result<()> {
        let current_hash = server.client.get_block_hash(block_height).await?;
        let mut last_history_id = server.db.last_history_id.get(()).unwrap_or_default();

        if let Some(cache) = reorg_cache.as_ref() {
            cache.lock().new_block(block_height, last_history_id);
        }

        server.db.block_hashes.set(block_height, current_hash);

        if reorg_cache.is_some() {
            debug!("Syncing block: {} ({})", current_hash, block_height);
        }

        let block = server.client.get_block(&current_hash).await?;
        let created = block.header.time;

        match server.addr_tx.send(server::threads::AddressesToLoad {
            height: block_height,
            addresses: block
                .txdata
                .iter()
                .flat_map(|x| &x.output)
                .map(|x| x.script_pubkey.clone())
                .collect(),
        }) {
            Ok(_) => {}
            _ => {
                if !server.token.is_cancelled() {
                    panic!("Failed to send addresses to load");
                }
            }
        }

        let prevouts = block
            .txdata
            .iter()
            .flat_map(|x| {
                let txid = x.txid();
                x.output.iter().enumerate().map(move |(idx, vout)| {
                    (
                        OutPoint {
                            txid,
                            vout: idx as u32,
                        },
                        vout,
                    )
                })
            })
            .filter(|x| !x.1.script_pubkey.is_provably_unspendable());

        server.db.prevouts.extend(prevouts);

        if block_height < *START_HEIGHT {
            server.db.last_block.set((), block_height);
            return Ok(());
        }

        if block.txdata.len() == 1 {
            server.db.last_block.set((), block_height);
            return server.new_hash(block_height, current_hash, &[]).await;
        }

        let mut token_cache = TokenCache::default();
        let prevouts = utils::load_prevouts_for_block(server.db.clone(), &block.txdata)?;

        if let Some(cache) = reorg_cache.as_ref() {
            prevouts.iter().for_each(|(key, value)| {
                cache.lock().removed_prevout(*key, value.clone());
            });
        }

        token_cache.valid_transfers.extend(
            server.db.load_transfers(
                prevouts
                    .iter()
                    .map(|(k, v)| AddressLocation {
                        address: v.script_pubkey.compute_script_hash(),
                        location: Location {
                            outpoint: *k,
                            offset: 0,
                        },
                    })
                    .collect(),
            ),
        );

        Self::parse_block(
            block_height,
            created,
            &block.txdata,
            &prevouts,
            &mut token_cache,
        );

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

    fn parse_inscriptions(payload: ParseInscription) -> Vec<InscriptionTemplate> {
        let mut result = vec![];

        for inscription in Inscription::from_transaction(payload.tx, payload.input_idx) {
            match inscription {
                ParsedInscription::None => {}
                ParsedInscription::Partial => {}
                ParsedInscription::Complete(inscription) => {
                    let genesis = {
                        InscriptionId {
                            txid: payload.tx.txid(),
                            index: *payload.inscription_idx,
                        }
                    };

                    *payload.inscription_idx += 1;

                    let content_type = inscription.content_type().map(|x| x.to_owned());

                    let pointer = inscription.pointer();

                    let mut inc = InscriptionTemplate {
                        content: inscription.into_body(),
                        content_type,
                        genesis,
                        location: Location {
                            offset: 0,
                            outpoint: OutPoint {
                                txid: payload.tx.txid(),
                                vout: payload.input_idx as u32,
                            },
                        },
                        owner: FullHash::ZERO,
                        value: 0,
                        leaked: false,
                    };

                    let Ok((mut vout, mut offset)) = InscriptionSearcher::get_output_index_by_input(
                        payload.inputs_cum.get(payload.input_idx).copied(),
                        &payload.tx.output,
                    ) else {
                        continue;
                    };

                    if let Ok((new_vout, new_offset)) =
                        InscriptionSearcher::get_output_index_by_input(pointer, &payload.tx.output)
                    {
                        vout = new_vout;
                        offset = new_offset;
                    }

                    let location: Location = Location {
                        outpoint: OutPoint {
                            txid: payload.tx.txid(),
                            vout,
                        },
                        offset,
                    };

                    let tx_out = &payload.tx.output[vout as usize];

                    if tx_out.script_pubkey.is_op_return() {
                        inc.owner = *OP_RETURN_HASH;
                    } else {
                        inc.owner = tx_out.script_pubkey.compute_script_hash();
                    }

                    inc.location = location;
                    inc.value = tx_out.value;

                    result.push(inc);
                }
            }
        }

        result
    }
}
