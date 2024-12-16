use axum::{
    response::{sse::Event, Sse},
    routing::post,
};
use futures::Stream;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use utils::to_scripthash;

use super::*;

mod utils;

type ApiResult<T> = core::result::Result<T, Response<String>>;
const INTERNAL: &str = "Can't handle request";

pub fn get_router(server: Arc<Server>) -> Router {
    Router::new()
        .route("/address/:address", get(address_tokens))
        .route("/address/:address/history", get(address_token_history))
        .route("/tokens", get(all_tokens))
        .route("/events", post(subscribe))
        .route("/status", get(status))
        .route("/proof-of-history", get(proof_of_history))
        .route("/events/:height", get(events_by_height))
        .route("/all-addresses", get(all_addresses))
        .with_state(server)
}

async fn all_addresses(State(server): State<Arc<Server>>) -> ApiResult<impl IntoResponse> {
    let (tx, rx) = tokio::sync::mpsc::channel(1000);
    tokio::spawn(async move {
        let addresses = server
            .db
            .address_token_to_balance
            .iter()
            .map(|x| x.0.address)
            .collect::<HashSet<_>>();

        let addresses = server
            .load_addresses(
                addresses.iter().copied(),
                *server.last_indexed_address_height.read().await,
            )
            .await
            .unwrap();

        for (_, address) in addresses {
            if tx.send(address).await.is_err() {
                break;
            }
        }
    });
    let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
    Ok(axum_streams::StreamBodyAs::json_array(stream))
}

async fn status(State(server): State<Arc<Server>>) -> ApiResult<impl IntoResponse> {
    let last_height = server
        .db
        .last_block
        .get(())
        .internal("Failed to get last height")?;
    let last_poh = server
        .db
        .proof_of_history
        .get(last_height)
        .internal("Failed to get last proof of history")?;
    let last_block_hash = server
        .db
        .block_hashes
        .get(last_height)
        .internal("Failed to get last block hash")?;

    let data = StatusRest {
        height: last_height,
        proof: last_poh.to_string(),
        blockhash: last_block_hash.to_string(),
    };

    Ok(Json(data))
}

async fn events_by_height(
    State(server): State<Arc<Server>>,
    Path(height): Path<u64>,
) -> ApiResult<impl IntoResponse> {
    let keys = server.db.block_events.get(height).unwrap_or_default();

    let mut res = Vec::<HistoryRest>::new();

    let iterator = server
        .db
        .address_token_to_history
        .multi_get(keys.iter())
        .into_iter()
        .zip(keys);

    for (v, k) in iterator {
        let v = v.not_found("No events found")?;
        res.push(
            HistoryRest::new(v.height, v.action, k, &server)
                .await
                .internal("Failed to load addresses")?,
        );
    }

    Ok(Json(res))
}

async fn proof_of_history(
    State(server): State<Arc<Server>>,
    Query(query): Query<ProofHistoryParams>,
) -> ApiResult<impl IntoResponse> {
    if let Some(limit) = query.limit {
        if limit > 100 {
            return Err("").bad_request("Limit exceeded");
        }
    }

    let res = server
        .db
        .proof_of_history
        .range(..&query.offset.map(|x| x).unwrap_or(u64::MAX), true)
        .map(|(height, hash)| ProofOfHistoryRest {
            hash: hash.to_string(),
            height,
        })
        .take(query.limit.unwrap_or(100))
        .collect_vec();

    Ok(Json(res))
}

async fn subscribe(
    State(server): State<Arc<Server>>,
    Json(payload): Json<SubscribeRequest>,
) -> ApiResult<Sse<impl Stream<Item = Result<Event, std::convert::Infallible>>>> {
    let (tx, rx) = mpsc::channel::<Result<Event, std::convert::Infallible>>(200_000);

    let addresses = payload.addresses.unwrap_or_default();

    let tokens = payload
        .tokens
        .unwrap_or_default()
        .into_iter()
        .map(|x| {
            let v: TokenTick = x.to_lowercase().as_bytes().try_into().anyhow()?;
            anyhow::Ok(String::from_utf8_lossy(&v).to_string())
        })
        .collect::<Result<HashSet<_>, _>>()
        .bad_request("Invalid token tick")?;

    {
        let mut rx = server.event_sender.subscribe();

        tokio::spawn(async move {
            while !server.token.is_cancelled() {
                match rx.try_recv() {
                    Ok(event) => {
                        match event {
                            ServerEvent::NewHistory(address_token, action) => {
                                if !addresses.is_empty()
                                    && !addresses.contains(&address_token.address)
                                {
                                    continue;
                                }

                                if !tokens.is_empty() && !tokens.contains(&address_token.token) {
                                    continue;
                                }

                                let data = Event::default().data(
                                    serde_json::to_string(&HistoryRest {
                                        address_token: address_token.into(),
                                        height: action.height,
                                        action: action.into(),
                                    })
                                    .unwrap(),
                                );

                                if tx.send(Ok(data)).await.is_err() {
                                    break;
                                };
                            }
                            ServerEvent::Reorg(blocks_count, new_height) => {
                                let data = Event::default().data(
                                    serde_json::to_string(&ReorgRest {
                                        event_type: "reorg".to_string(),
                                        blocks_count,
                                        new_height,
                                    })
                                    .unwrap(),
                                );

                                if tx.send(Ok(data)).await.is_err() {
                                    break;
                                };
                            }
                            ServerEvent::NewBlock(height, poh, blockhash) => {
                                let data = Event::default().data(
                                    serde_json::to_string(&NewBlockRest {
                                        event_type: "new_block".to_string(),
                                        height,
                                        proof: poh,
                                        blockhash,
                                    })
                                    .unwrap(),
                                );

                                if tx.send(Ok(data)).await.is_err() {
                                    break;
                                };
                            }
                        };
                    }
                    Err(tokio::sync::broadcast::error::TryRecvError::Lagged(count)) => {
                        error!("Lagged {} events. Disconnecting...", count);
                        break;
                    }
                    Err(tokio::sync::broadcast::error::TryRecvError::Closed) => {
                        break;
                    }
                    Err(tokio::sync::broadcast::error::TryRecvError::Empty) => {
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                }
            }
        });
    }

    let stream = ReceiverStream::new(rx);
    Ok(Sse::new(stream))
}

async fn address_token_history(
    State(server): State<Arc<Server>>,
    Path(script_str): Path<String>,
    Query(query): Query<AddressTokenHistoryParams>,
) -> ApiResult<impl IntoResponse> {
    let scripthash =
        to_scripthash("address", &script_str, *NETWORK).bad_request("Invalid address")?;

    if let Some(limit) = query.limit {
        if limit > 100 {
            return Err("").bad_request("Limit exceeded");
        }
    }
    let token: [u8; 4] = query
        .tick
        .as_bytes()
        .try_into()
        .bad_request("Invalid token length")?;

    let from = AddressTokenId {
        address: scripthash,
        id: 0,
        token,
    };

    let to = AddressTokenId {
        address: scripthash,
        id: query.offset.unwrap_or(u64::MAX),
        token,
    };

    let mut res = Vec::<HistoryRest>::new();

    for (k, v) in server
        .db
        .address_token_to_history
        .range(&from..&to, true)
        .take(query.limit.unwrap_or(100))
        .collect_vec()
    {
        res.push(
            HistoryRest::new(v.height, v.action, k, &server)
                .await
                .internal("Failed to load addresses")?,
        );
    }

    Ok(Json(res))
}

async fn address_tokens(
    State(server): State<Arc<Server>>,
    Path(script_str): Path<String>,
) -> ApiResult<Response<Body>> {
    let scripthash =
        to_scripthash("address", &script_str, *NETWORK).bad_request("Invalid address")?;

    let mut data = server
        .db
        .address_token_to_balance
        .range(
            &AddressToken {
                address: scripthash,
                token: [0; 4],
            }..=&AddressToken {
                address: scripthash,
                token: [u8::MAX; 4],
            },
            false,
        )
        .map(|(k, v)| TokenBalanceRest {
            tick: String::from_utf8_lossy(&k.token).to_string(),
            balance: v.balance,
            transferable_balance: v.transferable_balance,
            transfers_count: v.transfers_count,
            transfers: vec![],
        })
        .collect_vec();

    let mut transfers = HashMap::<TokenTick, Vec<(Location, TransferProto)>>::new();

    for (key, value) in server
        .db
        .address_location_to_transfer
        .range(
            &AddressLocation {
                address: scripthash,
                location: Location::zero(),
            }..,
            false,
        )
        .take_while(|x| x.0.address == scripthash)
    {
        transfers
            .entry(value.tick)
            .and_modify(|x| x.push((key.location, value.into())))
            .or_insert(vec![(key.location, value.into())]);
    }

    for token in data.iter_mut() {
        let tick: [u8; 4] = token.tick.as_bytes().try_into().unwrap();
        let transfers = transfers
            .remove(&tick)
            .unwrap_or_default()
            .into_iter()
            .map(|x| {
                let TransferProto::Bel20 { amt, .. } = x.1;
                TokenTransfer {
                    outpoint: x.0.outpoint.into(),
                    amount: amt,
                }
            })
            .collect();

        token.transfers = transfers;
    }

    let data = serde_json::to_vec(&data).internal(INTERNAL)?;

    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .header("X-Powered-By", "NINTONDO")
        .body(data.into())
        .internal(INTERNAL)
}

async fn all_tokens(State(server): State<Arc<Server>>) -> ApiResult<impl IntoResponse> {
    let data = server
        .db
        .token_to_meta
        .iter()
        .map(|(_, v)| TokenProtoRest::from(TokenMeta::from(v)))
        .collect_vec();

    Ok(Json(data))
}

#[derive(Deserialize)]
struct AddressTokenHistoryParams {
    offset: Option<u64>,
    limit: Option<usize>,
    tick: String,
}

#[derive(Deserialize)]
struct SubscribeRequest {
    #[serde(default)]
    addresses: Option<HashSet<String>>,
    #[serde(default)]
    tokens: Option<HashSet<String>>,
}

#[derive(Serialize)]
struct StatusRest {
    height: u64,
    proof: String,
    blockhash: String,
}

#[derive(Serialize)]
struct ProofOfHistoryRest {
    height: u64,
    hash: String,
}

#[derive(Deserialize)]
struct ProofHistoryParams {
    offset: Option<u64>,
    limit: Option<usize>,
}

#[derive(Serialize)]
struct ReorgRest {
    event_type: String,
    blocks_count: u32,
    new_height: u64,
}

#[derive(Serialize)]
struct NewBlockRest {
    event_type: String,
    height: u64,
    proof: sha256::Hash,
    blockhash: BlockHash,
}
