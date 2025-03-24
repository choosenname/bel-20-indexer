use super::*;

use dutils::async_thread::{Handler, Thread, ThreadController};

mod address_hash_saver;
pub mod blocks_loader;
mod event_sender;

pub use self::address_hash_saver::AddressesToLoad;

impl Server {
    pub async fn run_threads(
        self: Arc<Self>,
        token: WaitToken,
        addr_rx: kanal::Receiver<AddressesToLoad>,
        raw_event_tx: kanal::Receiver<RawServerEvent>,
        event_tx: tokio::sync::broadcast::Sender<ServerEvent>,
    ) -> anyhow::Result<()> {
        let addr_loader = ThreadController::new(address_hash_saver::AddressHasher {
            addr_rx,
            server: self.clone(),
            token: token.clone(),
        })
        .with_name("AddressHasher")
        .with_restart(Duration::from_secs(1))
        .with_cancellation(token.clone())
        .run();

        let event_sender = ThreadController::new(event_sender::EventSender {
            event_tx,
            raw_event_tx,
            server: self.clone(),
            token: token.clone(),
        })
        .with_name("EventSender")
        .with_restart(Duration::from_secs(1))
        .with_cancellation(token)
        .run();

        join_all(vec![addr_loader, event_sender])
            .await
            .into_iter()
            .try_collect()
            .anyhow()
    }
}
