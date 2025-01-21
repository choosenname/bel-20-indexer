use std::sync::Arc;

use super::{
    utils::{first_page, page_size_default, to_scripthash, validate_tick},
    AddressLocation, Fixed128, TransferProtoDB, BAD_PARAMS, INTERNAL, NETWORK,
};
use axum::{
    extract::{Path, Query, State},
    response::IntoResponse,
    Json,
};
use dutils::error::{ApiError, ContextWrapper};
use itertools::Itertools;
use nintypes::common::inscriptions::Outpoint;
use serde::{Deserialize, Serialize};
use validator::Validate;

use super::{ApiResult, Server, TokenTick, BAD_REQUEST, NOT_FOUND};

#[derive(Serialize, Deserialize)]
pub struct Token {
    pub height: u32,
    pub created: u32,
    pub tick: String,
    pub genesis: String,
    pub deployer: String,

    pub transactions: u32,
    pub holders: u32,
    pub supply: String,
    pub mint_percent: String,
    pub completed: bool,

    pub max: u64,
    pub lim: u64,
    pub dec: u8,
}

#[derive(Serialize, Deserialize, Default, Validate)]
pub struct TokenArgs {
    #[validate(custom(function = "validate_tick"))]
    pub tick: String,
}

pub async fn token(
    State(back): State<Arc<Server>>,
    Query(args): Query<TokenArgs>,
) -> ApiResult<impl IntoResponse> {
    args.validate().bad_request(BAD_REQUEST)?;
    let tick: TokenTick = args.tick.to_lowercase().as_bytes().try_into().unwrap();
    let token = back
        .db
        .token_to_meta
        .get(tick)
        .map(|v| Token {
            height: v.proto.height,
            created: v.proto.created,
            deployer: back
                .db
                .fullhash_to_address
                .get(v.proto.deployer)
                .unwrap_or("Not found".to_string()),
            transactions: v.proto.transactions,
            holders: back.holders.holders_by_tick(&tick).unwrap_or(0) as u32,
            tick: String::from_utf8_lossy(&tick).to_string(),
            genesis: v.genesis.to_string(),
            supply: v.proto.supply.to_string(),
            mint_percent: v.proto.mint_percent().to_string(),
            completed: v.proto.is_completed(),
            max: v.proto.max,
            lim: v.proto.lim,
            dec: v.proto.dec,
        })
        .not_found(NOT_FOUND)?;

    Ok(Json(token))
}

#[derive(Serialize, Deserialize)]
pub struct TokensResult {
    pub pages: usize,
    pub count: usize,
    pub tokens: Vec<Token>,
}

#[derive(Default, Serialize, Deserialize)]
pub enum SortBy {
    DeployTimeAsc,
    DeployTimeDesc,
    HoldersAsc,
    HoldersDesc,
    TransactionsAsc,
    #[default]
    TransactionsDesc,
}

#[derive(Default, Serialize, Deserialize)]
pub enum FilterBy {
    #[default]
    All,
    Completed,
    InProgress,
}

#[derive(Serialize, Deserialize, Default, Validate)]
pub struct TokensArgs {
    #[serde(default = "page_size_default")]
    #[validate(range(min = page_size_default(), max = 20))]
    pub page_size: usize,
    #[validate(range(min = 1))]
    #[serde(default = "first_page")]
    pub page: usize,
    #[serde(default)]
    pub sort_by: SortBy,
    #[serde(default)]
    pub filter_by: FilterBy,
    #[validate(length(min = 1, max = 4))]
    pub search: Option<String>,
}

pub async fn tokens(
    State(back): State<Arc<Server>>,
    Query(args): Query<TokensArgs>,
) -> ApiResult<impl IntoResponse> {
    args.validate().bad_request(BAD_PARAMS)?;
    let search = args.search.map(|x| x.to_lowercase().as_bytes().to_vec());
    // let tokens = backend.tokens.read();
    let tokens = &back.db.token_to_meta;

    let iter = tokens
        .iter()
        .filter(|x| match args.filter_by {
            FilterBy::All => true,
            FilterBy::Completed => x.1.is_completed(),
            FilterBy::InProgress => !x.1.is_completed(),
        })
        .filter(|x| match &search {
            Some(tick) => x.0.starts_with(tick),
            _ => true,
        });

    let stats = back.holders.stats();
    let all = match args.sort_by {
        SortBy::DeployTimeAsc => iter.sorted_by_key(|(_, v)| v.proto.created).collect_vec(),
        SortBy::DeployTimeDesc => iter
            .sorted_by_key(|(_, v)| v.proto.created)
            .rev()
            .collect_vec(),
        SortBy::HoldersAsc => iter.sorted_by_key(|(k, _)| stats.get(k)).collect_vec(),
        SortBy::HoldersDesc => iter
            .sorted_by_key(|(k, _)| stats.get(k))
            .rev()
            .collect_vec(),
        SortBy::TransactionsAsc => iter
            .sorted_by_key(|(_, v)| v.proto.transactions)
            .collect_vec(),
        SortBy::TransactionsDesc => iter
            .sorted_by_key(|(_, v)| v.proto.transactions)
            .rev()
            .collect_vec(),
    };

    let count = all.len();
    let pages = count.div_ceil(args.page_size);
    let tokens = all
        .iter()
        .skip((args.page - 1) * args.page_size)
        .take(args.page_size)
        .map(|(tick, v)| Token {
            height: v.proto.height,
            created: v.proto.created,
            mint_percent: v.proto.mint_percent().to_string(),
            tick: String::from_utf8_lossy(tick).to_string(),
            genesis: v.genesis.to_string(),
            deployer: back
                .db
                .fullhash_to_address
                .get(v.proto.deployer)
                .unwrap_or("Not found".to_string()),
            transactions: v.proto.transactions,
            holders: back.holders.holders_by_tick(tick).unwrap_or(0) as u32,
            supply: v.proto.supply.to_string(),
            completed: v.proto.is_completed(),
            max: v.proto.max,
            lim: v.proto.lim,
            dec: v.proto.dec,
        })
        .collect_vec();

    Ok(Json(TokensResult {
        count,
        pages,
        tokens,
    }))
}

pub async fn token_transfer_proof(
    State(state): State<Arc<Server>>,
    Path((address, outpoint)): Path<(String, Outpoint)>,
) -> ApiResult<impl IntoResponse> {
    let scripthash = to_scripthash("address", &address, *NETWORK).bad_request("Invalid address")?;

    let (from, to) = AddressLocation::search(scripthash, Some(outpoint.into())).into_inner();

    let data: Vec<_> = state
        .db
        .address_location_to_transfer
        .range(&from..&to, false)
        .map(|(_, TransferProtoDB { tick, amt })| {
            anyhow::Ok(TokenTransferProof {
                amt,
                tick: String::from_utf8(tick.to_vec()).anyhow()?,
            })
        })
        .try_collect()
        .track_with("")
        .internal(INTERNAL)?;

    Ok(Json(data))
}

#[derive(Serialize)]
pub struct TokenTransferProof {
    pub amt: Fixed128,
    pub tick: String,
}
