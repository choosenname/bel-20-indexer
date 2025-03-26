use super::*;

pub const PROTOCOL_ID: &[u8; 3] = b"ord";

mod envelope;
mod media;
pub mod parser;
mod searcher;
mod structs;
mod tag;
pub mod types;
mod utils;

use dutils::async_thread::Thread;
use electrs_client::{BlockMeta, UpdateCapable};
use envelope::{ParsedEnvelope, RawEnvelope};
use jsonrpc_async::client;
use searcher::InscriptionSearcher;
use structs::{Inscription, ParsedInscription};
use tag::Tag;
use types::{InscriptionsTokenHistory, TokenHistoryData};
pub use utils::ScriptToAddr;

pub use structs::Location;

pub async fn main_loop(token: WaitToken, server: Arc<Server>) -> anyhow::Result<()> {
    let reorg_cache = Arc::new(parking_lot::Mutex::new(reorg::ReorgCache::new()));
    let client = Arc::new(
        electrs_client::Client::<TokenHistoryData>::new_from_cfg(server.client.clone())
            .await
            .inspect_err(|e| {
                dbg!(e);
            })?,
    );

    loop {
        if let Err(e) = async {
            let last_electris_block = client.get_last_electrs_block_meta().await?;

            if let Some(block_number) = last_electris_block
                .height
                .checked_sub(reorg::REORG_CACHE_MAX_LEN as u32)
            {
                let end_block = client.get_electrs_block_meta(block_number).await?;
                initial_indexer(token.clone(), server.clone(), client.clone(), end_block).await?;
                return Ok(());
            }

            indexer(
                token.clone(),
                server.clone(),
                client.clone(),
                reorg_cache.clone(),
            )
            .await?;

            Ok::<(), anyhow::Error>(())
        }
        .await
        {
            error!("An error occurred: {:?}, retrying...", e);
            tokio::time::sleep(std::time::Duration::from_secs(300)).await;
            continue;
        }

        break;
    }

    info!("Server is finished");

    reorg_cache.lock().restore_all(&server).track().ok();

    server.db.flush_all();

    Ok(())
}

async fn initial_indexer(
    token: WaitToken,
    server: Arc<Server>,
    client: Arc<electrs_client::Client<TokenHistoryData>>,
    end: BlockMeta,
) -> anyhow::Result<()> {
    println!("Initial indexer");

    let blocks_storage = Arc::new(tokio::sync::Mutex::new(
        crate::server::threads::blocks_loader::LoadedBlocks::default(),
    ));

    let blocks_loader = dutils::async_thread::ThreadController::new(
        crate::server::threads::blocks_loader::BlocksLoader {
            storage: blocks_storage.clone(),
            client: client.clone(),
        },
    )
    .with_name("BlocksLoader")
    .with_restart(Duration::from_secs(5))
    .with_invoke_frq(Duration::from_secs(1))
    .with_cancellation(token.clone())
    .run();

    let last_electris_block = client.get_last_electrs_block_meta().await?;
    let block_number = server.db.last_block.get(()).unwrap_or_default(); // todo get real first block

    let progress = crate::utils::Progress::begin(
        "Indexing",
        last_electris_block.height as _,
        block_number as _,
    );

    let last_indexer_block = {
        let block_number = server.db.last_block.get(()).unwrap_or_default();
        client.get_electrs_block_meta(block_number).await?
    };

    blocks_storage
        .lock()
        .await
        .to_load(last_indexer_block.into());

    let mut sleep = token.repeat_until_cancel(Duration::from_secs(1));
    let mut is_reach_end = false;
    while !is_reach_end {
        let Some(blocks) = blocks_storage.lock().await.take_blocks() else {
            if !sleep.next().await || token.is_cancelled() {
                return Ok(());
            }

            continue;
        };

        let to_load = blocks
            .blocks
            .last()
            .map(|x| match x {
                electrs_client::Update::AddBlock { block, .. } => BlockHeader {
                    number: block.block_info.height,
                    hash: block.block_info.block_hash.into(),
                    prev_hash: block.block_info.prev_block_hash.into(),
                },
                _ => panic!("Got reorg wtf?"),
            })
            .clone();

        let mut updates = Vec::<types::ParsedTokenHistoryData>::new();

        for block in blocks.blocks {
            match block {
                electrs_client::Update::AddBlock { block, .. } => {
                    let casted_block: types::ParsedTokenHistoryData = block
                        .try_into()
                        .inspect_err(|e| {
                            dbg!(e);
                        })
                        .anyhow()?;

                    if casted_block.block_info.height == end.height {
                        is_reach_end = true;
                    }

                    updates.push(casted_block);
                }

                _ => unreachable!(),
            }
        }

        if !is_reach_end {
            blocks_storage
                .lock()
                .await
                .to_load(to_load.expect("Must exist blocks to load"));
        }

        let blocks_counter = updates.len();

        parser::InitialIndexer::handle_batch(updates, &server)
            .await
            .inspect_err(|e| {
                dbg!(e);
            })
            .track()
            .ok();

        progress.inc(blocks_counter as _);
    }

    blocks_loader.abort();

    Ok(())
}

async fn indexer(
    token: WaitToken,
    server: Arc<Server>,
    client: Arc<electrs_client::Client<TokenHistoryData>>,
    reorg_cache: Arc<parking_lot::Mutex<reorg::ReorgCache>>,
) -> anyhow::Result<()> {
    println!("Indexer");

    let last_indexer_block = {
        let block_number = server.db.last_block.get(()).unwrap_or_default();
        client.get_electrs_block_meta(block_number).await?
    };

    let mut repeater = token.repeat_until_cancel(Duration::from_secs(1));
    while repeater.next().await {
        let last_electris_block = client.get_last_electrs_block_meta().await?;

        let Some(blocks_gap) = last_electris_block
            .height
            .checked_sub(last_indexer_block.height)
        else {
            warn!(
                "Indexer has block number {} but got {}, sleep for a while ...",
                last_indexer_block.height, last_electris_block.height
            );
            continue;
        };

        let is_already_indexed_blocks =
            blocks_gap == 0 && last_electris_block.block_hash == last_indexer_block.block_hash;

        if is_already_indexed_blocks {
            info!("Indexer has the same block, sleep for a while ...");
            continue;
        }
        let blocks = reorg_cache.lock().get_blocks_headers();
        let Some(updates) = token.run_fn(load_blocks(&client, &blocks)).await else {
            break;
        };

        for update in updates? {
            if token.is_cancelled() {
                break;
            }

            handle_update(
                server.clone(),
                Some(reorg_cache.clone()),
                update,
                last_indexer_block.height,
            )
            .await?;
        }

        info!("Last block: {}", last_indexer_block.height);
    }

    info!("Server is finished");

    reorg_cache.lock().restore_all(&server).track().ok();

    server.db.flush_all();

    Ok(())
}

async fn handle_update(
    server: Arc<Server>,
    reorg_cache: Option<Arc<parking_lot::Mutex<reorg::ReorgCache>>>,
    update: electrs_client::Update<TokenHistoryData>,
    last_index_height: u32,
) -> anyhow::Result<u32> {
    let new_block_number = match update {
        electrs_client::Update::AddBlock { block, .. } => {
            let number = block.block_info.height;
            let token_history_data = block
                .try_into()
                .inspect_err(|e| {
                    dbg!(e);
                })
                .anyhow()?;
            parser::InitialIndexer::handle(token_history_data, server.clone(), reorg_cache)
                .await
                .inspect_err(|e| {
                    dbg!(e);
                })
                .track()
                .ok();
            number
        }
        electrs_client::Update::RemoveBlock { height }
        | electrs_client::Update::RemoveCachedBlock { height, .. } => {
            let reorg_counter = last_index_height - height;

            warn!(
                "Reorg detected: {} blocks, reorg height {}",
                reorg_counter, height
            );

            if let Some(cache) = reorg_cache {
                cache.lock().restore(&server, height).inspect_err(|e| {
                    dbg!(e);
                })?;
            }

            server
                .event_sender
                .send(ServerEvent::Reorg(reorg_counter, height))
                .ok();
            height - 1
        }
    };

    Ok(new_block_number)
}

async fn load_blocks(
    client: &electrs_client::Client<TokenHistoryData>,
    from: &[BlockHeader],
) -> anyhow::Result<Vec<electrs_client::Update<TokenHistoryData>>> {
    let from: Vec<_> = from.iter().map(|f| f.into()).collect();
    let updates = client
        .fetch_updates::<InscriptionsTokenHistory>(&from)
        .await
        .inspect_err(|e| {
            dbg!(e);
        })?;

    let (new_blocks, reorgs) = updates.iter().fold((0, 0), |(inserts, reorgs), v| match v {
        electrs_client::Update::AddBlock { .. } => (inserts + 1, reorgs),
        electrs_client::Update::RemoveBlock { .. } => (inserts, reorgs + 1),
        electrs_client::Update::RemoveCachedBlock { .. } => (inserts, reorgs + 1),
    });

    info!("Applying new blocks reorgs: {reorgs} new_blocks: {new_blocks}");

    Ok(updates)
}
