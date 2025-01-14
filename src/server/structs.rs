use super::*;

#[derive(Clone, Debug)]
pub enum ServerEvent {
    NewHistory(AddressTokenIdEvent, HistoryValueEvent),
    Reorg(u32, u64),
    NewBlock(u64, sha256::Hash, BlockHash),
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
    pub height: u64,
    pub action: TokenHistoryEvent,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub enum TokenHistoryEvent {
    Deploy {
        max: u64,
        lim: u64,
        dec: u8,
        txid: Txid,
    },
    Mint {
        amt: Fixed128,
        txid: Txid,
    },
    DeployTransfer {
        amt: Fixed128,
        txid: Txid,
    },
    Send {
        amt: Fixed128,
        recipient: String,
        txid: Txid,
    },
    Receive {
        amt: Fixed128,
        sender: String,
        txid: Txid,
    },
    SendReceive {
        amt: Fixed128,
        txid: Txid,
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
            } => Self::Deploy {
                max,
                lim,
                dec,
                txid,
            },
            TokenHistoryDB::Mint { amt, txid } => Self::Mint { amt, txid },
            TokenHistoryDB::DeployTransfer { amt, txid } => Self::DeployTransfer { amt, txid },
            TokenHistoryDB::Send {
                amt,
                recipient,
                txid,
            } => Self::Send {
                amt,
                recipient: addresses.get(&recipient).unwrap().clone(),
                txid,
            },
            TokenHistoryDB::Receive { amt, sender, txid } => Self::Receive {
                amt,
                sender: addresses.get(&sender).unwrap().clone(),
                txid,
            },
            TokenHistoryDB::SendReceive { amt, txid } => Self::SendReceive { amt, txid },
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
