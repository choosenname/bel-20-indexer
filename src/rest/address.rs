use std::{ops::RangeInclusive, sync::Arc};

use axum::{
    extract::{Path, Query, State},
    http::Uri,
    response::IntoResponse,
    Json,
};
use dutils::error::ApiError;
use itertools::Itertools;
use nintypes::common::inscriptions::Outpoint;
use serde::{Deserialize, Serialize};

use super::{
    utils::to_scripthash, AddressLocation, AddressToken, ApiResult, Fixed128, FullHash, Server,
    TokenTick, TokenTransfer, INTERNAL, NETWORK,
};

pub async fn address_tokens_tick(
    url: Uri,
    State(state): State<Arc<Server>>,
    Path(script_str): Path<String>,
) -> ApiResult<impl IntoResponse> {
    let script_type = url.path().split('/').nth(1).internal(INTERNAL)?;
    let scripthash =
        to_scripthash(script_type, &script_str, *NETWORK).bad_request("Invalid address")?;
    let (from, to) = AddressTick::search(scripthash).into_inner();
    let data = state
        .db
        .address_token_to_balance
        .range(&from..&to, false)
        .map(|x| String::from_utf8_lossy(&x.0.token).to_string())
        .collect_vec();
    Ok(Json(data))
}

#[cfg_attr(test, derive(Debug))]
#[derive(Clone, Copy, PartialEq, PartialOrd, Ord, Eq, Hash)]
pub struct AddressTick {
    pub address: FullHash,
    pub tick: TokenTick,
}

impl AddressTick {
    pub fn search(address: FullHash) -> RangeInclusive<AddressToken> {
        let start = AddressToken {
            address,
            token: [0; 4],
        };
        let end = AddressToken {
            address,
            token: [u8::MAX; 4],
        };

        start..=end
    }
}

pub async fn address_token_balance(
    url: Uri,
    State(state): State<Arc<Server>>,
    Path((script_str, tick)): Path<(String, String)>,
    Query(params): Query<AddressTokenBalanceArgs>,
) -> ApiResult<impl IntoResponse> {
    let script_type = url.path().split('/').nth(1).internal(INTERNAL)?;
    let scripthash =
        to_scripthash(script_type, &script_str, *NETWORK).bad_request("Invalid address")?;

    let tick: [u8; 4] = if let Ok(v) = tick.into_bytes().try_into() {
        v
    } else {
        Err("").bad_request("Invalid tick")?
    };

    let balance = state
        .db
        .address_token_to_balance
        .get(AddressToken {
            address: scripthash,
            token: tick,
        })
        .unwrap_or_default();

    let (from, to) =
        AddressLocation::search(scripthash, params.offset.map(|x| x.into())).into_inner();

    let transfers = state
        .db
        .address_location_to_transfer
        .range(&from..&to, false)
        .filter(|(_, v)| v.tick == tick)
        .map(|(k, v)| TokenTransfer {
            amount: v.amt,
            outpoint: k.location.outpoint,
        })
        .collect_vec();

    let data = TokenBalance {
        transfers,
        tick: String::from_utf8_lossy(&tick).to_string(),
        balance: balance.balance,
        transferable_balance: balance.transferable_balance,
        transfers_count: balance.transfers_count,
    };

    Ok(Json(data))
}

#[derive(Deserialize)]
pub struct AddressTokenBalanceArgs {
    pub offset: Option<Outpoint>,
}

#[derive(Serialize, Deserialize)]
pub struct TokenBalance {
    pub tick: String,
    pub balance: Fixed128,
    pub transferable_balance: Fixed128,
    pub transfers: Vec<TokenTransfer>,
    pub transfers_count: u64,
}
