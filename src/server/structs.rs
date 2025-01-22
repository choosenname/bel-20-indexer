use super::*;

#[derive(Clone, Debug)]
pub enum ServerEvent {
    NewHistory(AddressTokenIdEvent, HistoryValueEvent),
    Reorg(u32, u32),
    NewBlock(u32, sha256::Hash, BlockHash),
}

pub type RawServerEvent = Vec<(AddressTokenId, HistoryValue)>;

#[derive(Serialize, Deserialize, Clone, Debug, Hash, Eq, PartialEq, PartialOrd, Ord)]
pub struct AddressTokenIdEvent {
    pub address: String,
    pub token: String,
    pub id: u64,
}

#[derive(Serialize, Debug, Clone, Deserialize)]
pub struct HistoryValueEvent {
    pub height: u32,
    pub action: TokenHistoryEvent,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub enum TokenHistoryEvent {
    Deploy {
        max: u64,
        lim: u64,
        dec: u8,
        txid: Txid,
        vout: u32,
    },
    Mint {
        amt: Fixed128,
        txid: Txid,
        vout: u32,
    },
    DeployTransfer {
        amt: Fixed128,
        txid: Txid,
        vout: u32,
    },
    Send {
        amt: Fixed128,
        recipient: String,
        txid: Txid,
        vout: u32,
    },
    Receive {
        amt: Fixed128,
        sender: String,
        txid: Txid,
        vout: u32,
    },
    SendReceive {
        amt: Fixed128,
        txid: Txid,
        vout: u32,
    },
}

impl TokenHistoryEvent {
    fn into_event(value: TokenHistoryDB, addresses: &HashMap<FullHash, String>) -> Self {
        match value {
            TokenHistoryDB::Deploy {
                max,
                lim,
                dec,
                txid,
                vout,
            } => Self::Deploy {
                max,
                lim,
                dec,
                txid,
                vout,
            },
            TokenHistoryDB::Mint { amt, txid, vout } => Self::Mint { amt, txid, vout },
            TokenHistoryDB::DeployTransfer { amt, txid, vout } => {
                Self::DeployTransfer { amt, txid, vout }
            }
            TokenHistoryDB::Send {
                amt,
                recipient,
                txid,
                vout,
            } => Self::Send {
                amt,
                recipient: addresses.get(&recipient).unwrap().clone(),
                txid,
                vout,
            },
            TokenHistoryDB::Receive {
                amt,
                sender,
                txid,
                vout,
            } => Self::Receive {
                amt,
                sender: addresses.get(&sender).unwrap().clone(),
                txid,
                vout,
            },
            TokenHistoryDB::SendReceive { amt, txid, vout } => {
                Self::SendReceive { amt, txid, vout }
            }
        }
    }
}

impl HistoryValueEvent {
    pub fn into_event(value: HistoryValue, addresses: &HashMap<FullHash, String>) -> Self {
        Self {
            height: value.height,
            action: TokenHistoryEvent::into_event(value.action, addresses),
        }
    }
}
