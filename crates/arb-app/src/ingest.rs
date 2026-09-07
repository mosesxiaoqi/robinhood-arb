use arb_adapters::{
    rpc::{RpcSource, SourceError},
    store::{Store, StoreError},
};
use arb_core::types::{RawRecord, SourceCursor};
use std::{path::Path, thread};
use thiserror::Error;
use tokio::sync::{mpsc, oneshot};

#[derive(Debug, Error)]
pub enum IngestError {
    #[error(transparent)]
    Source(#[from] SourceError),
    #[error(transparent)]
    Storage(#[from] StoreError),
    #[error("ingest stopped: {0}")]
    Stopped(&'static str),
}
enum WriteCommand {
    Batch(
        Vec<RawRecord>,
        SourceCursor,
        oneshot::Sender<Result<(), StoreError>>,
    ),
    Gap(
        u64,
        String,
        u64,
        String,
        oneshot::Sender<Result<(), StoreError>>,
    ),
}

pub async fn collect(
    source: &RpcSource,
    path: &Path,
    from: u64,
    to: u64,
    queue_capacity: usize,
) -> Result<u64, IngestError> {
    if from > to || to == u64::MAX || queue_capacity == 0 || queue_capacity > 65536 {
        return Err(IngestError::Stopped("invalid range or queue capacity"));
    }
    let path = path.to_owned();
    let chain = source.chain_id();
    let source_label = source.source_label().to_owned();
    let (sender, mut receiver) = mpsc::channel::<WriteCommand>(queue_capacity);
    let (ready, initialized) = oneshot::channel();
    let worker = thread::spawn(move || {
        let mut store = match Store::open(&path) {
            Ok(s) => s,
            Err(e) => {
                let _ = ready.send(Err(e));
                return;
            }
        };
        if ready.send(store.cursor(chain, &source_label)).is_err() {
            return;
        }
        while let Some(command) = receiver.blocking_recv() {
            let (result, ack) = match command {
                WriteCommand::Batch(records, cursor, ack) => {
                    (store.append_raw(&records, &cursor), ack)
                }
                WriteCommand::Gap(chain, source, block, reason, ack) => {
                    (store.record_gap(chain, &source, block, &reason), ack)
                }
            };
            let failed = result.is_err();
            let _ = ack.send(result);
            if failed {
                break;
            }
        }
    });
    let result = async {
        let previous = initialized
            .await
            .map_err(|_| IngestError::Stopped("writer startup"))??;
        if previous.as_ref().is_some_and(|c| from > c.next_block) {
            return Err(IngestError::Stopped("requested start skips stored cursor"));
        }
        let start = previous.as_ref().map_or(from, |c| c.next_block);
        let mut last_hash = previous.and_then(|c| c.last_block_hash);
        let mut count = 0;
        for number in start..=to {
            let records = match source.fetch_block(number).await {
                Ok(records) => records,
                Err(error) => {
                    let (ack, done) = oneshot::channel();
                    sender
                        .send(WriteCommand::Gap(
                            chain,
                            source.source_label().into(),
                            number,
                            error.to_string(),
                            ack,
                        ))
                        .await
                        .map_err(|_| IngestError::Stopped("writer closed"))?;
                    done.await
                        .map_err(|_| IngestError::Stopped("gap not acknowledged"))??;
                    return Err(error.into());
                }
            };
            let block = records
                .first()
                .ok_or(IngestError::Stopped("empty block batch"))?;
            let position = block
                .position
                .as_ref()
                .ok_or(IngestError::Stopped("missing block position"))?;
            if let Some(expected) = last_hash {
                let value: serde_json::Value = serde_json::from_slice(&block.payload)
                    .map_err(|_| IngestError::Stopped("invalid stored block JSON"))?;
                if value["result"]["parentHash"].as_str() != Some(format!("{expected:#x}").as_str())
                {
                    let (ack, done) = oneshot::channel();
                    sender
                        .send(WriteCommand::Gap(
                            chain,
                            source.source_label().into(),
                            number,
                            "parent hash changed; recovery required".into(),
                            ack,
                        ))
                        .await
                        .map_err(|_| IngestError::Stopped("writer closed"))?;
                    done.await
                        .map_err(|_| IngestError::Stopped("gap not acknowledged"))??;
                    return Err(IngestError::Stopped(
                        "parent hash changed; recovery required",
                    ));
                }
            }
            let block_hash = position.block_hash;
            let cursor = SourceCursor {
                chain_id: chain,
                source: source.source_label().into(),
                next_block: number + 1,
                last_block_hash: Some(block_hash),
            };
            let (ack, committed) = oneshot::channel();
            sender
                .send(WriteCommand::Batch(records, cursor, ack))
                .await
                .map_err(|_| IngestError::Stopped("writer closed"))?;
            committed
                .await
                .map_err(|_| IngestError::Stopped("write not acknowledged"))??;
            last_hash = Some(block_hash);
            count += 1;
        }
        Ok(count)
    }
    .await;
    drop(sender);
    tokio::task::spawn_blocking(move || worker.join())
        .await
        .map_err(|_| IngestError::Stopped("writer join failed"))?
        .map_err(|_| IngestError::Stopped("writer panicked"))?;
    result
}
