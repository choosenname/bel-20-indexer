use super::*;

use super::proto::*;
use super::structs::*;

type Tickers = HashSet<TokenTick>;
type Users = HashSet<(FullHash, TokenTick)>;

#[derive(Clone, Serialize, Deserialize, Debug)]
pub enum HistoryTokenAction {
    Deploy {
        tick: TokenTick,
        max: u64,
        lim: u64,
        dec: u8,
        recipient: FullHash,
    },
    Mint {
        tick: TokenTick,
        amt: Decimal,
        recipient: FullHash,
    },
    DeployTransfer {
        tick: TokenTick,
        amt: Decimal,
        recipient: FullHash,
    },
    Send {
        tick: TokenTick,
        amt: Decimal,
        recipient: FullHash,
        sender: FullHash,
    },
}

impl HistoryTokenAction {
    pub fn tick(&self) -> TokenTick {
        match self {
            HistoryTokenAction::Deploy { tick, .. } => tick.clone(),
            HistoryTokenAction::Mint { tick, .. } => tick.clone(),
            HistoryTokenAction::DeployTransfer { tick, .. } => tick.clone(),
            HistoryTokenAction::Send { tick, .. } => tick.clone(),
        }
    }

    pub fn recipient(&self) -> FullHash {
        match self {
            HistoryTokenAction::Mint { recipient, .. } => *recipient,
            HistoryTokenAction::DeployTransfer { recipient, .. } => *recipient,
            HistoryTokenAction::Send { recipient, .. } => *recipient,
            HistoryTokenAction::Deploy { recipient, .. } => *recipient,
        }
    }

    pub fn sender(&self) -> Option<FullHash> {
        match self {
            HistoryTokenAction::Send { sender, .. } => Some(*sender),
            _ => None,
        }
    }
}

#[derive(Clone, Default)]
pub struct TokenCache {
    /// All tokens. Used to check if a transfer is valid. Used like a cache, loaded from db before parsing.
    pub tokens: HashMap<TokenTick, TokenMeta>,

    /// All token accounts. Used to check if a transfer is valid. Used like a cache, loaded from db before parsing.
    pub token_accounts: HashMap<AddressToken, TokenBalance>,

    /// All token actions that are not validated yet but just parsed.
    pub token_actions: Vec<TokenAction>,

    /// All transfer actions. Used to check if a transfer is valid. Used like cache.
    pub all_transfers: HashMap<Location, TransferProto>,

    /// All transfer actions that are valid. Used to write to the db.
    pub valid_transfers: BTreeMap<Location, (FullHash, TransferProto)>,
}
impl TokenCache {
    fn try_parse(content_type: &str, content: &[u8]) -> Result<Brc4, Brc4ParseErr> {
        match content_type.split(';').nth(0) {
            Some("text/plain" | "application/json") => {
                let Ok(data) = String::from_utf8(content.to_vec()) else {
                    return Err(Brc4ParseErr::InvalidUtf8);
                };

                let brc4 = match serde_json::from_str::<Brc4>(&data) {
                    Ok(b) => b,
                    Err(error) => match error.to_string().as_str() {
                        "Invalid decimal: empty" => return Err(Brc4ParseErr::DecimalEmpty),
                        "Invalid decimal: overflow from too many digits" => {
                            return Err(Brc4ParseErr::DecimalOverflow)
                        }
                        "value cannot start from + or -" => {
                            return Err(Brc4ParseErr::DecimalPlusMinus)
                        }
                        "value cannot start or end with ." => {
                            return Err(Brc4ParseErr::DecimalDotStartEnd)
                        }
                        "value cannot contain spaces" => return Err(Brc4ParseErr::DecimalSpaces),
                        "invalid digit found in string" => return Err(Brc4ParseErr::InvalidDigit),
                        _msg => {
                            // eprintln!("ERR: {msg:?}");
                            return Err(Brc4ParseErr::Unknown);
                        }
                    },
                };

                match &brc4 {
                    Brc4::Mint {
                        proto: MintProto::Bel20 { amt, .. },
                    } if !amt.is_zero() => Ok(brc4),
                    Brc4::Deploy {
                        proto: DeployProto::Bel20 { dec, lim, max, .. },
                    } if *dec <= DeployProto::MAX_DEC && !lim.is_zero() && !max.is_zero() => {
                        Ok(brc4)
                    }
                    Brc4::Transfer {
                        proto: TransferProto::Bel20 { amt, .. },
                    } if !amt.is_zero() => Ok(brc4),
                    _ => Err(Brc4ParseErr::WrongProtocol),
                }
            }
            _ => Err(Brc4ParseErr::WrongContentType),
        }
    }

    /// Parse token action from InscriptionTemplace and returns bool if it is mint or not.
    pub fn parse_token_action(&mut self, inc: &InscriptionTemplate) -> Option<TransferProto> {
        if inc.owner.is_op_return_hash() {
            return None;
        }

        let Ok(brc4) = Self::try_parse(inc.content_type.as_ref()?, inc.content.as_ref()?) else {
            return None;
        };

        // skip to not add invalid token creation in token_cache
        if inc.leaked {
            return None;
        }

        match brc4 {
            Brc4::Deploy { proto } => {
                self.token_actions.push(TokenAction::Deploy {
                    genesis: inc.genesis,
                    proto: proto.into(),
                    owner: inc.owner,
                });
            }
            Brc4::Mint { proto } => {
                self.token_actions.push(TokenAction::Mint {
                    owner: inc.owner.clone(),
                    proto,
                });
            }
            Brc4::Transfer { proto } => {
                self.token_actions.push(TokenAction::Transfer {
                    location: inc.location,
                    owner: inc.owner.clone(),
                    proto: proto.clone(),
                });
                self.all_transfers.insert(inc.location, proto);
                return Some(proto);
            }
        };

        None
    }

    pub fn trasferred(&mut self, location: Location, recipient: FullHash) {
        self.token_actions.push(TokenAction::Transferred {
            transfer_location: location,
            recipient: Some(recipient),
        });
    }

    pub fn burned_transfer(&mut self, location: Location) {
        self.token_actions.push(TokenAction::Transferred {
            transfer_location: location,
            recipient: None,
        });
    }

    pub fn load_tokens_data(&mut self, db: &DB) -> anyhow::Result<()> {
        let (tickers, users) = self.fill_tickers_and_users();

        self.tokens = db
            .token_to_meta
            .multi_get(tickers.iter().collect_vec())
            .into_iter()
            .zip(tickers)
            .filter_map(|(v, k)| v.map(|x| (k, TokenMeta::from(x))))
            .collect::<HashMap<_, _>>();

        self.token_accounts = db.load_token_accounts(users);

        Ok(())
    }

    fn fill_tickers_and_users(&mut self) -> (Tickers, Users) {
        let mut tickers = HashSet::new();
        let mut users = HashSet::new();

        for action in &self.token_actions {
            match action {
                TokenAction::Deploy {
                    proto: DeployProtoDB { tick, .. },
                    ..
                } => {
                    // Load ticks because we need to check if tick is deployed
                    tickers.insert(*tick);
                }
                TokenAction::Mint {
                    owner,
                    proto: MintProto::Bel20 { tick, .. },
                    ..
                } => {
                    tickers.insert(*tick);
                    users.insert((owner.clone(), tick.clone()));
                }
                TokenAction::Transfer {
                    owner,
                    proto: TransferProto::Bel20 { tick, .. },
                    ..
                } => {
                    tickers.insert(*tick);
                    users.insert((owner.clone(), tick.clone()));
                }
                TokenAction::Transferred {
                    transfer_location,
                    recipient,
                } => {
                    let valid_transfer = self.valid_transfers.get(&transfer_location);
                    let proto = {
                        self.all_transfers
                            .get(&transfer_location)
                            .map(|x| Some(x.clone()))
                            .unwrap_or_else(|| {
                                valid_transfer.map(|x| Some(x.1.clone())).unwrap_or(None)
                            })
                    };
                    if let Some(TransferProto::Bel20 { tick, .. }) = proto {
                        if let Some(recipient) = recipient {
                            users.insert((recipient.clone(), tick.clone()));
                            if let Some(transfer) = valid_transfer {
                                users.insert((transfer.0.clone(), tick.clone()));
                            }
                            tickers.insert(tick.clone());
                        }
                    }
                }
            }
        }
        (tickers, users)
    }

    pub fn process_token_actions(
        &mut self,
        reorg_cache: Option<Arc<parking_lot::Mutex<crate::reorg::ReorgCache>>>,
    ) -> Vec<HistoryTokenAction> {
        let mut history = vec![];

        for action in self.token_actions.drain(..) {
            match action {
                TokenAction::Deploy {
                    genesis,
                    proto,
                    owner,
                } => {
                    let DeployProtoDB {
                        tick,
                        max,
                        lim,
                        dec,
                        ..
                    } = proto.clone();
                    match self.tokens.entry(tick.clone()) {
                        std::collections::hash_map::Entry::Vacant(e) => {
                            e.insert(TokenMeta { genesis, proto });

                            history.push(HistoryTokenAction::Deploy {
                                tick,
                                max,
                                lim,
                                dec,
                                recipient: owner,
                            });

                            reorg_cache.as_ref().map(|x| {
                                x.lock().added_deployed_token(tick);
                            });
                        }
                        _ => {}
                    }
                }
                TokenAction::Mint { owner, proto } => {
                    let MintProto::Bel20 { tick, amt } = proto;
                    let Some(token) = self.tokens.get_mut(&tick) else {
                        continue;
                    };
                    let DeployProtoDB {
                        max,
                        lim,
                        dec,
                        supply,
                        mint_count,
                        ..
                    } = &mut token.proto;

                    if amt.scale() > *dec as u32 {
                        continue;
                    }

                    if Decimal::from_u64(*lim).unwrap() < amt {
                        continue;
                    }

                    if *supply == Decimal::from_u64(*max).unwrap() {
                        continue;
                    }
                    let amt = amt.min(Decimal::from_u64(*max).unwrap() - *supply);
                    *supply += amt;

                    let key = AddressToken {
                        address: owner,
                        token: tick.clone(),
                    };
                    self.token_accounts.entry(key.clone()).or_default().balance += amt;
                    *mint_count += 1;

                    history.push(HistoryTokenAction::Mint {
                        tick,
                        amt,
                        recipient: key.address.clone(),
                    });

                    reorg_cache.as_ref().map(|x| {
                        x.lock().added_minted_token(key, amt);
                    });
                }
                TokenAction::Transfer {
                    owner,
                    location,
                    proto,
                } => {
                    let Some(data) = self.all_transfers.remove(&location) else {
                        // skip cause is it transfer already spent
                        continue;
                    };

                    let TransferProto::Bel20 { tick, amt } = proto.clone();

                    let Some(token) = self.tokens.get_mut(&tick) else {
                        continue;
                    };
                    let DeployProtoDB {
                        transfer_count,
                        dec,
                        ..
                    } = &mut token.proto;

                    if amt.scale() > *dec as u32 {
                        // skip wrong protocol
                        continue;
                    }

                    let key = AddressToken {
                        address: owner,
                        token: tick,
                    };
                    let Some(account) = self.token_accounts.get_mut(&key) else {
                        continue;
                    };

                    if amt > account.balance {
                        continue;
                    }

                    reorg_cache.as_ref().map(|x| {
                        x.lock().added_transfer_token(location, key.clone(), amt);
                    });

                    account.balance -= amt;
                    account.transfers_count += 1;
                    account.transferable_balance += amt;

                    history.push(HistoryTokenAction::DeployTransfer {
                        tick: key.token,
                        amt,
                        recipient: key.address.clone(),
                    });

                    self.valid_transfers.insert(location, (key.address, data));
                    *transfer_count += 1;
                }
                TokenAction::Transferred {
                    transfer_location,
                    recipient,
                } => {
                    let Some((sender, TransferProto::Bel20 { tick, amt })) =
                        self.valid_transfers.remove(&transfer_location)
                    else {
                        // skip cause transfer has been already spent
                        continue;
                    };

                    if !self.tokens.contains_key(&tick) {
                        unreachable!();
                    }

                    if let Some(recipient) = recipient {
                        let key = AddressToken {
                            address: recipient.clone(),
                            token: tick.clone(),
                        };

                        self.token_accounts.entry(key).or_default().balance += amt;

                        let old_key = AddressToken {
                            address: sender.clone(),
                            token: tick.clone(),
                        };

                        let old_account = self.token_accounts.get_mut(&old_key).unwrap();
                        if old_account.transfers_count == 0
                            || old_account.transferable_balance < amt
                        {
                            panic!("Invalid transfer sender balance");
                        }
                        old_account.transfers_count -= 1;
                        old_account.transferable_balance -= amt;

                        history.push(HistoryTokenAction::Send {
                            amt,
                            tick,
                            recipient: recipient.clone(),
                            sender: sender.clone(),
                        });
                    } else {
                        let old_key = AddressToken {
                            address: sender.clone(),
                            token: tick.clone(),
                        };

                        let old_account = self.token_accounts.get_mut(&old_key).unwrap();
                        if old_account.transfers_count == 0
                            || old_account.transferable_balance < amt
                        {
                            panic!("Invalid transfer sender balance");
                        }
                        old_account.transfers_count -= 1;
                        old_account.transferable_balance -= amt;

                        history.push(HistoryTokenAction::Send {
                            tick,
                            amt,
                            recipient: sender.clone(),
                            sender: sender.clone(),
                        });
                    }

                    reorg_cache.as_ref().map(|x| {
                        x.lock().removed_transfer_token(
                            AddressLocation {
                                address: sender,
                                location: transfer_location.into(),
                            },
                            TransferProto::Bel20 { tick, amt },
                            recipient,
                        );
                    });
                }
            }
        }

        history
    }

    async fn _write_tokens_amount(
        token_accounts: Vec<(AddressToken, TokenBalance)>,
        db: Arc<DB>,
    ) -> anyhow::Result<()> {
        if !token_accounts.is_empty() {
            db.address_token_to_balance
                .extend(token_accounts.into_iter())
        }
        Ok(())
    }

    async fn _write_tokens_meta(
        tokens: Vec<(TokenTick, TokenMeta)>,
        db: Arc<DB>,
    ) -> anyhow::Result<()> {
        if !tokens.is_empty() {
            db.token_to_meta
                .extend(tokens.into_iter().map(|(k, v)| (k, TokenMetaDB::from(v))));
        }

        Ok(())
    }

    pub async fn write_token_data(&mut self, db: Arc<DB>) -> anyhow::Result<()> {
        let (a, b) = futures::future::join(
            Self::_write_tokens_meta(self.tokens.drain().collect_vec(), db.clone()).spawn(),
            Self::_write_tokens_amount(self.token_accounts.drain().collect_vec(), db).spawn(),
        )
        .await;

        a.anyhow()?.anyhow()?;
        b.anyhow()?.anyhow()?;

        Ok(())
    }

    pub fn write_valid_transfers(self, db: &DB) -> anyhow::Result<()> {
        if !self.valid_transfers.is_empty() {
            db.address_location_to_transfer
                .extend(
                    self.valid_transfers
                        .into_iter()
                        .map(|(location, (address, proto))| {
                            (
                                AddressLocation {
                                    address,
                                    location: location.into(),
                                },
                                TransferProtoDB::from(proto),
                            )
                        }),
                );
        }

        Ok(())
    }
}
