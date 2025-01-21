use super::*;

#[derive(Eq, PartialEq, Clone, Ord, PartialOrd, Serialize, Deserialize, Debug)]
pub struct SortedByBalance(pub Fixed128, pub FullHash);

pub struct Holders {
    balances: parking_lot::RwLock<HashMap<TokenTick, BTreeSet<SortedByBalance>>>,
    stats: parking_lot::RwLock<HashMap<TokenTick, usize>>,
}

enum Action {
    Increase,
    Decrease,
}

impl Holders {
    pub fn init(db: &DB) -> Self {
        let holders = HashMap::from_iter(
            db.address_token_to_balance
                .iter()
                .filter(|(_, v)| !v.balance.is_zero() || !v.transferable_balance.is_zero())
                .map(|(k, v)| {
                    (
                        k.token,
                        SortedByBalance(v.balance + v.transferable_balance, k.address),
                    )
                })
                .sorted_unstable_by_key(|x| x.0)
                .chunk_by(|(k, _)| *k)
                .into_iter()
                .map(|(k, v)| (k, v.map(|(_, v)| v).collect::<BTreeSet<_>>())),
        );

        let stats = holders.iter().map(|x| (*x.0, x.1.len())).collect();

        Self {
            balances: parking_lot::RwLock::new(holders),
            stats: parking_lot::RwLock::new(stats),
        }
    }

    // pub fn get_balances_len(&self) -> usize {
    //     self.balances.read().len()
    // }

    pub fn get_balances(&self) -> parking_lot::lock_api::RwLockReadGuard<'_, parking_lot::RawRwLock, HashMap<[u8; 4], BTreeSet<SortedByBalance>>> {
        self.balances.read()
    }
    // pub fn get(
    //     &self,
    //     tick: TokenTick,
    //     offset: usize,
    //     limit: usize,
    // ) -> HashSet<crate::rest::Holder> {
    //     let holders = self.balances.read();
    //
    //     let result = match holders.get(&tick) {
    //         Some(data) => {
    //             let count = data.len();
    //             let pages = count.div_ceil(limit);
    //             let mut holders = Vec::with_capacity(limit);
    //
    //             let keys = data
    //                 .iter()
    //                 .rev()
    //                 .enumerate()
    //                 .skip(offset)
    //                 .take(limit)
    //                 .map(|(rank, x)| (rank + 1, x.0, x.1));
    //
    //             let address_rtx = &backend.db.fullhash_to_address;
    //
    //             for (rank, balance, hash) in keys {
    //                 let address = address_rtx.get(hash)?;
    //                 let percent =
    //                     balance.into_decimal() * Decimal::new(100, 0) / supply.into_decimal();
    //
    //                 holders.push(Holder {
    //                     rank,
    //                     address,
    //                     balance: balance.to_string(),
    //                     percent: percent.to_string(),
    //                 })
    //             }
    //
    //             Holders {
    //                 pages,
    //                 count,
    //                 max_percent,
    //                 holders,
    //             }
    //         }
    //         _ => Holders {
    //             max_percent: "0".to_string(),
    //             ..Default::default()
    //         },
    //     };
    // }
    // pub fn get(&self, tick: TokenTick, offset: usize, limit: usize) -> HashSet<FullHash> {
    //     let balances = self.balances.read();
    //
    //     balances
    //         .get(&tick)
    //         .map(|x| {
    //             x.iter()
    //                 .skip(offset)
    //                 .take(limit)
    //                 .cloned()
    //                 .map(|x| x.1)
    //                 .collect()
    //         })
    //         .unwrap_or_default()
    // }

    /// hack because i cant throw -amt cause of type
    pub fn decrease(&self, key: AddressToken, prev_balance: &TokenBalance, amt: Fixed128) {
        self.change(key, prev_balance, amt, Action::Decrease);
    }

    /// returns `true` if new holder was created
    pub fn increase(&self, key: AddressToken, prev_balance: &TokenBalance, amt: Fixed128) {
        self.change(key, prev_balance, amt, Action::Increase)
    }

    pub fn holders_by_tick(&self, tick: &TokenTick) -> Option<usize> {
        self.stats.read().get(tick).cloned()
    }

    pub fn stats(&self) -> HashMap<TokenTick, usize> {
        self.stats.read().clone()
    }

    fn change(&self, key: AddressToken, acc: &TokenBalance, amt: Fixed128, action: Action) {
        // used to prevent footgun with balance (not to forget to add transferable)
        let old_balance = acc.balance + acc.transferable_balance;
        let mut balances = self.balances.write();

        let v = balances.entry(key.token).or_default();

        let existed = v.remove(&SortedByBalance(old_balance, key.address));

        match action {
            Action::Increase => {
                if !existed {
                    self.stats
                        .write()
                        .entry(key.token)
                        .and_modify(|x| *x += 1)
                        .or_insert(1);
                }

                v.insert(SortedByBalance(old_balance + amt, key.address));
            }
            Action::Decrease => {
                let bal = old_balance - amt;
                if !bal.is_zero() {
                    v.insert(SortedByBalance(bal, key.address));
                } else {
                    self.stats.write().entry(key.token).and_modify(|x| *x -= 1);
                }
            }
        }
    }
}
