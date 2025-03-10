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

use envelope::{ParsedEnvelope, RawEnvelope};
use searcher::InscriptionSearcher;
use structs::{Inscription, ParsedInscription};
use tag::Tag;
use types::{InscriptionsTokenHistory, TokenHistoryData};
pub use utils::ScriptToAddr;

pub use structs::Location;

pub async fn main_loop(token: WaitToken, server: Arc<Server>) -> anyhow::Result<()> {
    let reorg_cache = Arc::new(parking_lot::Mutex::new(reorg::ReorgCache::new()));
    let mut last_block_height = 0;
    
    let client =
        electrs_client::Client::<TokenHistoryData>::new_from_cfg(server.client.clone()).await?;

    let mut repeater = token.repeat_until_cancel(Duration::from_millis(50));

    while repeater.next().await {
        tokio::time::sleep(Duration::from_secs(1)).await;

        // Index new blocks
        let updates = client
            .fetch_updates_from_reorgs::<InscriptionsTokenHistory>()
            .await?;

        let (new_blocks, reorgs) = updates.iter().fold((0, 0), |(inserts, reorgs), v| match v {
            electrs_client::Update::AddBlock { .. } => (inserts + 1, reorgs),
            electrs_client::Update::RemoveBlock { .. } => (inserts, reorgs + 1),
            electrs_client::Update::RemoveCachedBlock { .. } => (inserts, reorgs + 1),
        });

        info!("Applying new blocks reorgs: {reorgs} new_blocks: {new_blocks}");

        for update in updates {
            match update {
                electrs_client::Update::AddBlock { block, .. } => {
                    last_block_height = block.block_info.height;
                    
                    parser::InitialIndexer::handle(
                        block,
                        server.clone(),
                        Some(reorg_cache.clone()),
                    )
                        .await
                        .track()
                        .ok();
                }
                electrs_client::Update::RemoveBlock { height }
                | electrs_client::Update::RemoveCachedBlock { height, .. } => {
                    let reorg_counter = last_block_height - height;

                    warn!("Reorg detected: {} blocks", reorg_counter);
                    server
                        .event_sender
                        .send(ServerEvent::Reorg(reorg_counter, height))
                        .ok();
                    reorg_cache.lock().restore(&server, height)?;
                }
            }
        }
    }

    info!("Server is finished");

    reorg_cache.lock().restore_all(&server).track().ok();

    server.db.flush_all();

    Ok(())
}
