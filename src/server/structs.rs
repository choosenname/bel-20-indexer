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
    Deploy { max: u64, lim: u64, dec: u8 },
    Mint { amt: Decimal },
    DeployTransfer { amt: Decimal },
    Send { amt: Decimal, recipient: String },
    Receive { amt: Decimal, sender: String },
    SendReceive { amt: Decimal },
}

impl TokenHistoryEvent {
    fn into_event(value: TokenHistoryDB, addresses: &HashMap<FullHash, String>) -> Self {
        match value {
            TokenHistoryDB::Deploy { max, lim, dec } => Self::Deploy { max, lim, dec },
            TokenHistoryDB::Mint { amt } => Self::Mint { amt },
            TokenHistoryDB::DeployTransfer { amt } => Self::DeployTransfer { amt },
            TokenHistoryDB::Send { amt, recipient } => Self::Send {
                amt,
                recipient: addresses.get(&recipient).unwrap().clone(),
            },
            TokenHistoryDB::Receive { amt, sender } => Self::Receive {
                amt,
                sender: addresses.get(&sender).unwrap().clone(),
            },
            TokenHistoryDB::SendReceive { amt } => Self::SendReceive { amt },
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
