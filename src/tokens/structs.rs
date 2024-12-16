use bellscoin::consensus;
use server::{AddressTokenIdEvent, HistoryValueEvent};

use super::*;

#[derive(Serialize, Deserialize, Clone, Debug, Hash, Eq, PartialEq, PartialOrd, Ord)]
pub struct AddressToken {
    pub address: FullHash,
    pub token: TokenTick,
}

impl From<AddressTokenId> for AddressToken {
    fn from(value: AddressTokenId) -> Self {
        Self {
            address: value.address,
            token: value.token,
        }
    }
}

impl db::Pebble for AddressToken {
    type Inner = Self;

    fn from_bytes(v: Cow<[u8]>) -> anyhow::Result<Self::Inner> {
        Ok(Self {
            address: v[..32].try_into().anyhow()?,
            token: v[32..].try_into().anyhow()?,
        })
    }

    fn get_bytes(v: &Self::Inner) -> Cow<[u8]> {
        let mut result = Vec::with_capacity(32 + 4);
        result.extend(v.address);
        result.extend(v.token);
        Cow::Owned(result)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Hash, Eq, PartialEq, PartialOrd, Ord)]
pub struct AddressTokenId {
    pub address: FullHash,
    pub token: TokenTick,
    pub id: u64,
}

impl db::Pebble for AddressTokenId {
    type Inner = Self;

    fn get_bytes(v: &Self::Inner) -> Cow<[u8]> {
        let mut result = Vec::with_capacity(32 + 4 + 8);
        result.extend(v.address);
        result.extend(v.token);
        result.extend(v.id.to_be_bytes());

        Cow::Owned(result)
    }

    fn from_bytes(v: Cow<[u8]>) -> anyhow::Result<Self::Inner> {
        let address: FullHash = v[..32].try_into().anyhow()?;
        let token = v[32..32 + 4].try_into().anyhow()?;
        let id = u64::from_be_bytes(v[32 + 4..].try_into().anyhow()?);

        Ok(Self { address, id, token })
    }
}

impl db::Pebble for Vec<AddressTokenId> {
    type Inner = Self;

    fn get_bytes(v: &Self::Inner) -> Cow<[u8]> {
        let mut result = Vec::new();
        for item in v {
            result.extend(AddressTokenId::get_bytes(item).into_owned());
        }
        Cow::Owned(result)
    }

    fn from_bytes(v: Cow<[u8]>) -> anyhow::Result<Self::Inner> {
        v.chunks(32 + 4 + 8)
            .into_iter()
            .map(|x| AddressTokenId::from_bytes(Cow::Borrowed(x)))
            .collect()
    }
}

#[derive(Debug, Serialize, Deserialize, PartialOrd, Ord, PartialEq, Eq, Clone, Default)]
pub struct TokenBalance {
    pub balance: Decimal,
    pub transferable_balance: Decimal,
    pub transfers_count: u64,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub enum TokenHistoryDB {
    Deploy { max: u64, lim: u64, dec: u8 },
    Mint { amt: Decimal },
    DeployTransfer { amt: Decimal },
    Send { amt: Decimal, recipient: FullHash },
    Receive { amt: Decimal, sender: FullHash },
    SendReceive { amt: Decimal },
}

#[derive(Serialize, Debug, Clone, Deserialize)]
pub struct HistoryValue {
    pub height: u64,
    pub action: TokenHistoryDB,
}

impl TokenHistoryDB {
    pub fn from_token_history(token_history: HistoryTokenAction) -> Self {
        match token_history {
            HistoryTokenAction::Deploy { max, lim, dec, .. } => {
                TokenHistoryDB::Deploy { max, lim, dec }
            }
            HistoryTokenAction::Mint { amt, .. } => TokenHistoryDB::Mint { amt },
            HistoryTokenAction::DeployTransfer { amt, .. } => {
                TokenHistoryDB::DeployTransfer { amt }
            }
            HistoryTokenAction::Send {
                amt,
                recipient,
                sender,
                ..
            } => {
                if sender == recipient {
                    TokenHistoryDB::SendReceive { amt }
                } else {
                    TokenHistoryDB::Send { amt, recipient }
                }
            }
        }
    }

    pub fn address(&self) -> Option<&FullHash> {
        match self {
            TokenHistoryDB::Receive { sender, .. } => Some(sender),
            TokenHistoryDB::Send { recipient, .. } => Some(recipient),
            _ => None,
        }
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct TokenBalanceRest {
    pub tick: String,
    pub balance: Decimal,
    pub transferable_balance: Decimal,
    pub transfers: Vec<TokenTransfer>,
    pub transfers_count: u64,
}

#[derive(Serialize, Deserialize)]
pub struct TokenProtoRest {
    pub genesis: InscriptionId,
    pub tick: String,
    pub max: u64,
    pub lim: u64,
    pub dec: u8,
    pub supply: Decimal,
    pub mint_count: u64,
    pub transfer_count: u64,
}

impl From<TokenMeta> for TokenProtoRest {
    fn from(value: TokenMeta) -> Self {
        let result = match value.proto {
            DeployProtoDB {
                tick,
                max,
                lim,
                dec,
                supply,
                mint_count,
                transfer_count,
            } => Self {
                genesis: value.genesis,
                tick: String::from_utf8_lossy(&tick).to_string(),
                max,
                lim,
                dec,
                supply,
                mint_count,
                transfer_count,
            },
        };
        result
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Hash, PartialEq, PartialOrd, Ord, Eq)]
pub struct AddressLocation {
    pub address: FullHash,
    pub location: Location,
}

impl db::Pebble for AddressLocation {
    type Inner = Self;

    fn get_bytes(v: &Self::Inner) -> Cow<[u8]> {
        let mut result = Vec::with_capacity(32 + 44);

        result.extend(v.address);

        result.extend(consensus::serialize(&v.location.outpoint));
        result.extend(v.location.offset.to_be_bytes());

        Cow::Owned(result)
    }

    fn from_bytes(v: Cow<[u8]>) -> anyhow::Result<Self::Inner> {
        let address = v[..32].try_into().anyhow()?;
        let outpoint: OutPoint = consensus::deserialize(&v[32..32 + 36])?;
        let offset = u64::from_be_bytes(v[32 + 32 + 4..].try_into().anyhow()?);

        Ok(Self {
            address,
            location: Location { outpoint, offset },
        })
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Brc4ActionErr {
    NotDeployed,
    AlreadyDeployed,
    ReachDecBound,
    ReachLimBound,
    SupplyMinted,
    InsufficientBalance,
    Transferred,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Brc4ParseErr {
    WrongContentType,
    WrongProtocol,
    DecimalEmpty,
    DecimalOverflow,
    DecimalPlusMinus,
    DecimalDotStartEnd,
    DecimalSpaces,
    InvalidDigit,
    InvalidUtf8,
    Unknown,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Brc4Error {
    Action(Brc4ActionErr),
    Parse(Brc4ParseErr),
}

pub type TokenTick = [u8; 4];

#[derive(Debug, PartialEq, Copy, Clone, Hash, Eq)]
pub struct InscriptionId {
    pub txid: Txid,
    pub index: u32,
}

impl<'de> Deserialize<'de> for InscriptionId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(DeserializeFromStr::deserialize(deserializer)?.0)
    }
}

impl Serialize for InscriptionId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_str(self)
    }
}

impl Display for InscriptionId {
    fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
        write!(f, "{}i{}", self.txid, self.index)
    }
}

impl From<InscriptionId> for OutPoint {
    fn from(val: InscriptionId) -> Self {
        OutPoint {
            txid: val.txid,
            vout: val.index,
        }
    }
}

impl From<OutPoint> for InscriptionId {
    fn from(outpoint: OutPoint) -> Self {
        Self {
            txid: outpoint.txid,
            index: outpoint.vout,
        }
    }
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub enum TokenAction {
    /// Deploy new token action.
    Deploy {
        genesis: InscriptionId,
        proto: DeployProtoDB,
        owner: FullHash,
    },
    /// Mint new token action.
    Mint { owner: FullHash, proto: MintProto },
    /// Transfer token action.
    Transfer {
        location: Location,
        owner: FullHash,
        proto: TransferProto,
    },
    /// Founded move of transfer action.
    Transferred {
        transfer_location: Location,
        recipient: Option<FullHash>,
    },
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub enum ParsedTokenAction {
    Deploy {
        tick: TokenTick,
        max: u64,
        lim: u64,
        dec: u8,
    },
    Mint {
        tick: TokenTick,
        amt: Decimal,
    },
    DeployTransfer {
        tick: TokenTick,
        amt: Decimal,
    },
    SpentTransfer {
        tick: TokenTick,
        amt: Decimal,
    },
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TokenTransfer {
    pub outpoint: OutPoint,
    pub amount: Decimal,
}

#[derive(Serialize)]
#[serde(tag = "type")]
pub enum TokenActionRest {
    Deploy { max: u64, lim: u64, dec: u8 },
    Mint { amt: Decimal },
    DeployTransfer { amt: Decimal },
    Send { amt: Decimal, recipient: String },
    Receive { amt: Decimal, sender: String },
    SendReceive { amt: Decimal },
}

impl From<HistoryValueEvent> for TokenActionRest {
    fn from(value: HistoryValueEvent) -> Self {
        match value.action {
            server::TokenHistoryEvent::Deploy { max, lim, dec } => Self::Deploy { max, lim, dec },
            server::TokenHistoryEvent::DeployTransfer { amt } => Self::DeployTransfer { amt },
            server::TokenHistoryEvent::Mint { amt } => Self::Mint { amt },
            server::TokenHistoryEvent::Send { amt, recipient } => Self::Send { amt, recipient },
            server::TokenHistoryEvent::Receive { amt, sender } => Self::Receive { amt, sender },
            server::TokenHistoryEvent::SendReceive { amt } => Self::SendReceive { amt },
        }
    }
}

impl TokenActionRest {
    fn from_with_addresses(value: TokenHistoryDB, addresses: &HashMap<FullHash, String>) -> Self {
        match value {
            TokenHistoryDB::Deploy { max, lim, dec } => TokenActionRest::Deploy { max, lim, dec },
            TokenHistoryDB::Mint { amt } => TokenActionRest::Mint { amt },
            TokenHistoryDB::DeployTransfer { amt } => TokenActionRest::DeployTransfer { amt },
            TokenHistoryDB::Send { amt, recipient } => TokenActionRest::Send {
                amt,
                recipient: addresses.get(&recipient).unwrap().clone(),
            },
            TokenHistoryDB::Receive { amt, sender } => TokenActionRest::Receive {
                amt,
                sender: addresses.get(&sender).unwrap().clone(),
            },
            TokenHistoryDB::SendReceive { amt } => TokenActionRest::SendReceive { amt },
        }
    }
}

#[derive(Serialize)]
pub struct AddressTokenIdRest {
    pub id: u64,
    pub address: String,
    pub tick: String,
}

impl From<AddressTokenIdEvent> for AddressTokenIdRest {
    fn from(value: AddressTokenIdEvent) -> Self {
        Self {
            address: value.address,
            id: value.id,
            tick: value.token,
        }
    }
}

#[derive(Serialize)]
pub struct HistoryRest {
    #[serde(flatten)]
    pub address_token: AddressTokenIdRest,
    pub height: u64,
    #[serde(flatten)]
    pub action: TokenActionRest,
}

impl HistoryRest {
    pub async fn new(
        height: u64,
        action: TokenHistoryDB,
        address_token: AddressTokenId,
        server: &Server,
    ) -> anyhow::Result<Self> {
        let keys = [action.address().copied(), Some(address_token.address)]
            .into_iter()
            .flatten();

        let addresses = server.load_addresses(keys, height).await?;

        Ok(Self {
            height,
            action: TokenActionRest::from_with_addresses(action, &addresses),
            address_token: AddressTokenIdRest {
                address: addresses.get(&address_token.address).unwrap().clone(),
                id: address_token.id,
                tick: String::from_utf8_lossy(&address_token.token).to_string(),
            },
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenMeta {
    pub genesis: InscriptionId,
    pub proto: DeployProtoDB,
}

#[derive(Serialize, Deserialize, Debug, Copy, Clone)]
pub struct TokenMetaDB {
    pub genesis: InscriptionId,
    pub proto: DeployProtoDB,
}

impl From<TokenMeta> for TokenMetaDB {
    fn from(meta: TokenMeta) -> Self {
        TokenMetaDB {
            genesis: meta.genesis,
            proto: meta.proto.into(),
        }
    }
}

impl From<TokenMetaDB> for TokenMeta {
    fn from(meta: TokenMetaDB) -> Self {
        TokenMeta {
            genesis: meta.genesis,
            proto: meta.proto.into(),
        }
    }
}

#[derive(Clone)]
pub struct InscriptionTemplate {
    pub genesis: InscriptionId,
    pub location: Location,
    pub content_type: Option<String>,
    pub owner: FullHash,
    pub value: u64,
    pub content: Option<Vec<u8>>,
    pub leaked: bool,
}

pub(crate) struct DeserializeFromStr<T: FromStr>(pub(crate) T);

impl<'de, T: FromStr> Deserialize<'de> for DeserializeFromStr<T>
where
    T::Err: Display,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(Self(
            FromStr::from_str(&String::deserialize(deserializer)?)
                .map_err(serde::de::Error::custom)?,
        ))
    }
}

#[derive(Debug)]
pub enum ParseError {
    Character(char),
    Length(usize),
    Separator(char),
    Txid(bellscoin::hashes::hex::Error),
    Index(std::num::ParseIntError),
}

impl Display for ParseError {
    fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
        match self {
            Self::Character(c) => write!(f, "invalid character: '{c}'"),
            Self::Length(len) => write!(f, "invalid length: {len}"),
            Self::Separator(c) => write!(f, "invalid separator: `{c}`"),
            Self::Txid(err) => write!(f, "invalid txid: {err}"),
            Self::Index(err) => write!(f, "invalid index: {err}"),
        }
    }
}

impl std::error::Error for ParseError {}

impl FromStr for InscriptionId {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Some(char) = s.chars().find(|char| !char.is_ascii()) {
            return Err(ParseError::Character(char));
        }

        const TXID_LEN: usize = 64;
        const MIN_LEN: usize = TXID_LEN + 2;

        if s.len() < MIN_LEN {
            return Err(ParseError::Length(s.len()));
        }

        let txid = &s[..TXID_LEN];

        let separator = s
            .chars()
            .nth(TXID_LEN)
            .ok_or_else(|| ParseError::Separator(' '))?;

        if separator != 'i' {
            return Err(ParseError::Separator(separator));
        }

        let vout = &s[TXID_LEN + 1..];

        Ok(Self {
            txid: txid.parse().map_err(ParseError::Txid)?,
            index: vout.parse().map_err(ParseError::Index)?,
        })
    }
}
