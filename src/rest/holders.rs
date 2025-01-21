use std::sync::Arc;

use axum::{
    extract::{Query, State},
    response::IntoResponse,
    Json,
};
use dutils::error::ApiError;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use validator::Validate;

use super::{
    utils::{first_page, page_size_default, validate_tick},
    ApiResult, Fixed128, Server, TokenTick, BAD_PARAMS, INTERNAL,
};

pub async fn holders(
    State(backend): State<Arc<Server>>,
    Query(args): Query<Args>,
) -> ApiResult<impl IntoResponse> {
    args.validate().bad_request(BAD_PARAMS)?;

    let tick: TokenTick = args.tick.as_bytes().try_into().unwrap();
    let supply = backend.db.token_to_meta.get(tick).map(|x| x.proto.supply);
    let holders = backend.holders.get_balances();

    let result = match (supply, holders.get(&tick)) {
        (Some(supply), Some(data)) => {
            let count = data.len();
            let pages = count.div_ceil(args.page_size);
            let mut holders = Vec::with_capacity(args.page_size);
            let max_percent = data
                .last()
                .map(|x| (x.0 * Fixed128::from(100)).into_decimal() / supply.into_decimal())
                .map(|x| x.to_string())
                .unwrap_or("0".to_string());

            let keys = data
                .iter()
                .rev()
                .enumerate()
                .skip((args.page - 1) * args.page_size)
                .take(args.page_size)
                .map(|(rank, x)| (rank + 1, x.0, x.1));

            let address_rtx = &backend.db.fullhash_to_address;

            for (rank, balance, hash) in keys {
                let address = address_rtx.get(hash).internal(INTERNAL)?;
                let percent = balance.into_decimal() * Decimal::new(100, 0) / supply.into_decimal();

                holders.push(Holder {
                    rank,
                    address,
                    balance: balance.to_string(),
                    percent: percent.to_string(),
                })
            }

            Holders {
                pages,
                count,
                max_percent,
                holders,
            }
        }
        _ => Holders {
            max_percent: "0".to_string(),
            ..Default::default()
        },
    };

    Ok(Json(result))
}

#[derive(Serialize, Deserialize, Default, Validate)]
pub struct Args {
    #[serde(default = "page_size_default")]
    #[validate(range(min = page_size_default(), max = 20))]
    pub page_size: usize,
    #[validate(range(min = 1))]
    #[serde(default = "first_page")]
    pub page: usize,
    #[validate(custom(function = "validate_tick"))]
    pub tick: String,
}

#[derive(Serialize, Deserialize)]
pub struct Holder {
    pub rank: usize,
    pub address: String,
    pub balance: String,
    pub percent: String,
}

#[derive(Serialize, Deserialize, Default)]
pub struct Holders {
    pub pages: usize,
    pub count: usize,
    pub max_percent: String,
    pub holders: Vec<Holder>,
}
