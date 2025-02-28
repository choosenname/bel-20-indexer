use bellscoin::BlockHash;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct TokenHistoryData {
    pub block_info: BlockInfo,
    pub token_history: Vec<TokenHistory>,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq, Hash)]
pub struct TokenHistory {
    pub from: String,
    pub to: String,
    pub from_location: HistoryLocation,
    pub to_location: HistoryLocation,
    pub leaked: bool,
    pub token: ParsedTokenAction,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct InscriptionsTokenHistory {
    pub data: Vec<(BlockInfo, Vec<TokenHistory>)>,
    pub reorg: Vec<u32>,
}

#[derive(Clone, Serialize, Deserialize, Debug, Copy)]
pub struct BlockInfo {
    pub height: u32,
    pub created: u32,
    pub block_hash: BlockHash,
    pub prev_block_hash: BlockHash,
}

#[derive(
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Debug,
    Clone,
    Copy,
    bytemuck::Pod,
    bytemuck::Zeroable,
    Serialize,
    Deserialize,
    Hash,
)]
#[repr(C, packed)]
pub struct HistoryLocation {
    pub offset: u64,
    pub outpoint: Outpoint,
}

#[derive(
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Debug,
    Clone,
    bytemuck::Pod,
    bytemuck::Zeroable,
    Copy,
    Hash,
    Serialize,
    Deserialize,
)]
#[repr(C, packed)]
pub struct Outpoint {
    pub txid: [u8; 32],
    pub vout: u32,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum ParsedTokenAction {
    Deploy {
        tick: TokenTick,
        max: Fixed128,
        lim: Fixed128,
        dec: u8,
    },
    Mint {
        tick: TokenTick,
        amt: Fixed128,
    },
    DeployTransfer {
        tick: TokenTick,
        amt: Fixed128,
    },
    SpentTransfer {
        tick: TokenTick,
        amt: Fixed128,
    },
}

#[derive(Clone, Serialize, Deserialize, PartialEq, PartialOrd, Ord, Eq, Hash, Debug)]
pub struct TokenTick(pub Vec<u8>);

pub type Fixed128 = nintypes::utils::fixed::Fixed128<18>;
