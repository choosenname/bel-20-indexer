use super::*;
use std::collections::VecDeque;

#[derive(Default)]
pub struct LoadedBlocks {
    pub to_load: VecDeque<BlockHeader>,
    pub blocks: VecDeque<Blocks>,
}

pub struct Blocks {
    pub from: BlockHeader,
    pub blocks: Vec<electrs_client::Update<inscriptions::types::TokenHistoryData>>,
}

impl LoadedBlocks {
    pub fn get_to_load(&self) -> Option<BlockHeader> {
        self.to_load.front().cloned()
    }

    pub fn to_load(&mut self, from: BlockHeader) {
        self.to_load.push_back(from);
    }

    pub fn take_blocks(&mut self) -> Option<Blocks> {
        self.blocks.pop_front()
    }
}

#[derive(Clone)]
pub struct BlocksLoader {
    pub storage: Arc<tokio::sync::Mutex<LoadedBlocks>>,
    pub client: Arc<electrs_client::Client<inscriptions::types::TokenHistoryData>>,
}

impl Handler for BlocksLoader {
    async fn run(&mut self) -> anyhow::Result<()> {
        let Some(to_load) = self.storage.lock().await.get_to_load() else {
            return Ok(());
        };

        let blocks = Self::fetch_blocks(&self.client, to_load).await.anyhow()?;

        let mut storage = self.storage.lock().await;
        storage.to_load.pop_front();
        storage.blocks.push_back(blocks);

        Ok(())
    }
}

impl BlocksLoader {
    async fn fetch_blocks(
        client: &electrs_client::Client<inscriptions::types::TokenHistoryData>,
        from: BlockHeader,
    ) -> electrs_client::ClientResult<Blocks> {
        let casted_from: Vec<_> = vec![(&from).into()];
        let blocks = client
            .fetch_updates::<inscriptions::types::InscriptionsTokenHistory>(&casted_from)
            .await
            .inspect_err(|e| {
                dbg!(e);
            })?;

        let (new_blocks, reorgs) = blocks.iter().fold((0, 0), |(inserts, reorgs), v| match v {
            electrs_client::Update::AddBlock { .. } => (inserts + 1, reorgs),
            electrs_client::Update::RemoveBlock { .. } => (inserts, reorgs + 1),
            electrs_client::Update::RemoveCachedBlock { .. } => (inserts, reorgs + 1),
        });

        info!("Applying new blocks reorgs: {reorgs} new_blocks: {new_blocks}");

        Ok(Blocks { from, blocks })
    }
}
