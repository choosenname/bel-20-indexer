use super::*;

use serde::de::Error;

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Protocol(pub Brc4Value, pub Option<Brc4ActionErr>);

pub fn bel_20_decimal<'de, D>(deserializer: D) -> Result<Fixed128, D::Error>
where
    D: Deserializer<'de>,
{
    let val = <&str>::deserialize(deserializer)?;

    if val.starts_with('+') | val.starts_with('-') {
        return Err(Error::custom("value cannot start from + or -"));
    }

    if val.starts_with('.') | val.ends_with('.') {
        return Err(Error::custom("value cannot start or end with ."));
    }

    if val.starts_with(' ') | val.ends_with(' ') {
        return Err(Error::custom("value cannot contain spaces"));
    }

    Fixed128::from_str(val).map_err(Error::custom)
}

pub fn bel_20_tick<'de, D>(deserializer: D) -> Result<TokenTick, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let val = <&str as serde::Deserialize>::deserialize(deserializer)?.to_lowercase();
    val.as_bytes().try_into().map_err(Error::custom)
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "op")]
#[serde(rename_all = "lowercase")]
pub enum Brc4 {
    Mint {
        #[serde(flatten)]
        proto: MintProto,
    },
    Deploy {
        #[serde(flatten)]
        proto: DeployProto,
    },
    Transfer {
        #[serde(flatten)]
        proto: TransferProto,
    },
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "p")]
#[serde_as]
pub enum MintProto {
    #[serde(rename = "bel-20")]
    Bel20 {
        #[serde(deserialize_with = "bel_20_tick")]
        tick: TokenTick,
        #[serde(deserialize_with = "bel_20_decimal")]
        amt: Fixed128,
    },
}
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "p")]
#[serde_as]
pub enum DeployProto {
    #[serde(rename = "bel-20")]
    Bel20 {
        #[serde(deserialize_with = "bel_20_tick")]
        tick: TokenTick,
        #[serde(with = ":: serde_with :: As :: < DisplayFromStr >")]
        max: u64,
        #[serde(with = ":: serde_with :: As :: < DisplayFromStr >")]
        lim: u64,
        #[serde(with = ":: serde_with :: As :: < DisplayFromStr >")]
        #[serde(default = "DeployProto::default_dec")]
        dec: u8,
    },
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
pub struct DeployProtoDB {
    pub tick: TokenTick,
    pub max: u64,
    pub lim: u64,
    pub dec: u8,
    pub supply: Fixed128,
    pub transfer_count: u64,
    pub mint_count: u64,
}

impl From<DeployProto> for DeployProtoDB {
    fn from(value: DeployProto) -> Self {
        match value {
            DeployProto::Bel20 {
                tick,
                max,
                lim,
                dec,
            } => DeployProtoDB {
                tick,
                max,
                lim,
                dec,
                supply: Fixed128::ZERO,
                transfer_count: 0,
                mint_count: 0,
            },
        }
    }
}

impl DeployProto {
    pub const DEFAULT_DEC: u8 = 18;
    pub const MAX_DEC: u8 = 18;
    pub fn default_dec() -> u8 {
        Self::DEFAULT_DEC
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
#[serde(tag = "p")]
#[serde_as]
pub enum TransferProto {
    #[serde(rename = "bel-20")]
    Bel20 {
        #[serde(deserialize_with = "bel_20_tick")]
        tick: TokenTick,
        #[serde(deserialize_with = "bel_20_decimal")]
        amt: Fixed128,
    },
}

#[derive(Serialize, Deserialize, Clone, Copy)]
pub struct TransferProtoDB {
    pub tick: TokenTick,
    pub amt: Fixed128,
}

impl From<TransferProto> for TransferProtoDB {
    fn from(value: TransferProto) -> Self {
        match value {
            TransferProto::Bel20 { tick, amt } => TransferProtoDB { tick, amt },
        }
    }
}

impl From<TransferProtoDB> for TransferProto {
    fn from(value: TransferProtoDB) -> Self {
        TransferProto::Bel20 {
            tick: value.tick,
            amt: value.amt,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
pub enum Brc4Value {
    Mint {
        tick: TokenTick,
        amt: Fixed128,
    },
    Transfer {
        tick: TokenTick,
        amt: Fixed128,
    },
    Deploy {
        tick: TokenTick,
        max: u64,
        lim: u64,
        dec: u8,
    },
}

impl From<&DeployProto> for Brc4Value {
    fn from(v: &DeployProto) -> Self {
        match v {
            DeployProto::Bel20 {
                tick,
                max,
                lim,
                dec,
                ..
            } => Brc4Value::Deploy {
                tick: *tick,
                max: *max,
                lim: *lim,
                dec: *dec,
            },
        }
    }
}

impl From<&MintProto> for Brc4Value {
    fn from(v: &MintProto) -> Self {
        match v {
            MintProto::Bel20 { tick, amt } => Brc4Value::Mint {
                tick: *tick,
                amt: *amt,
            },
        }
    }
}

impl From<&TransferProto> for Brc4Value {
    fn from(v: &TransferProto) -> Self {
        match v {
            TransferProto::Bel20 { tick, amt } => Brc4Value::Transfer {
                tick: *tick,
                amt: *amt,
            },
        }
    }
}
