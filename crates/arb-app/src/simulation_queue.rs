use alloy_primitives::{B256, keccak256};
use arb_adapters::{rpc::RpcSource, store::Store};
use arb_core::{research::Candidate, simulation::*};
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::{
    sync::mpsc,
    task::{JoinHandle, JoinSet},
};
#[derive(Clone)]
pub struct QueueOptions {
    pub queue_id: String,
    pub capacity: usize,
    pub concurrency: usize,
    pub queue_timeout_ms: u64,
    pub call_timeout_ms: u64,
}
#[derive(Debug, thiserror::Error)]
#[error("simulation queue: {0}")]
pub struct QueueError(pub String);
fn error(e: impl std::fmt::Display) -> QueueError {
    QueueError(e.to_string())
}
fn now() -> Result<u64, QueueError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(error)?
        .as_millis()
        .try_into()
        .map_err(error)
}
struct Pending {
    record: SimulationRecord,
    queued: Instant,
}
pub struct SimulationQueue {
    sender: Option<mpsc::Sender<Pending>>,
    worker: Option<JoinHandle<Result<(), QueueError>>>,
    path: PathBuf,
    queue_id: String,
}
impl Drop for SimulationQueue {
    fn drop(&mut self) {
        if let Some(worker) = &self.worker {
            worker.abort();
        }
    }
}
async fn insert(path: PathBuf, record: SimulationRecord) -> Result<bool, QueueError> {
    tokio::task::spawn_blocking(move || Store::open(&path)?.insert_simulation(&record))
        .await
        .map_err(error)?
        .map_err(error)
}
async fn save(path: PathBuf, mut record: SimulationRecord) -> Result<(), QueueError> {
    tokio::task::spawn_blocking(move || Store::open(&path)?.save_simulation(&mut record))
        .await
        .map_err(error)?
        .map_err(error)
}
impl SimulationQueue {
    pub fn new(
        source: Arc<RpcSource>,
        path: PathBuf,
        options: QueueOptions,
    ) -> Result<Self, QueueError> {
        if options.queue_id.is_empty()
            || options.queue_id.len() > 128
            || options.queue_id.chars().any(char::is_control)
            || !(1..=65536).contains(&options.capacity)
            || !(1..=64).contains(&options.concurrency)
            || !(1..=300000).contains(&options.queue_timeout_ms)
            || !(1..=300000).contains(&options.call_timeout_ms)
        {
            return Err(QueueError("invalid bounds".into()));
        }
        let (sender, receiver) = mpsc::channel(options.capacity);
        let worker = tokio::spawn(dispatch(source, path.clone(), options.clone(), receiver));
        Ok(Self {
            sender: Some(sender),
            worker: Some(worker),
            path,
            queue_id: options.queue_id,
        })
    }
    pub async fn submit(
        &self,
        candidate: Candidate,
        request: SimulationRequest,
    ) -> Result<B256, QueueError> {
        let queued = Instant::now();
        let submitted = now()?;
        let id = keccak256(
            serde_json::to_vec(&(&self.queue_id, candidate.id, &request)).map_err(error)?,
        );
        let reserve = self
            .sender
            .as_ref()
            .ok_or_else(|| QueueError("closed".into()))?
            .clone()
            .try_reserve_owned();
        let (permit, phase, message) = match reserve {
            Ok(permit) => (Some(permit), SimulationPhase::Queued, None),
            Err(mpsc::error::TrySendError::Full(_)) => (
                None,
                SimulationPhase::QueueFull,
                Some("bounded simulation queue full".into()),
            ),
            Err(mpsc::error::TrySendError::Closed(_)) => (
                None,
                SimulationPhase::Interrupted,
                Some("simulation dispatcher stopped".into()),
            ),
        };
        let accepted = permit.is_some();
        let record = SimulationRecord {
            id,
            queue_id: self.queue_id.clone(),
            candidate_id: candidate.id,
            run_id: candidate.run_id,
            view_id: candidate.view_id,
            expected_position: candidate.opportunity.position,
            request,
            phase,
            outcome: if accepted {
                SimulationOutcome::Unknown
            } else {
                SimulationOutcome::Unavailable
            },
            submitted_at_ms: submitted,
            queued_at_ms: accepted.then_some(submitted),
            started_at_ms: None,
            ended_at_ms: (!accepted).then_some(submitted),
            queue_wait_ns: None,
            elapsed_ns: None,
            result: None,
            error: message,
            error_evidence: vec![],
            canonical: false,
            validates_original_candidate: false,
        };
        if insert(self.path.clone(), record.clone()).await?
            && let Some(permit) = permit
        {
            permit.send(Pending { record, queued });
        }
        Ok(id)
    }
    pub async fn shutdown(mut self, timeout_ms: u64) -> Result<(), QueueError> {
        self.sender.take();
        let mut worker = self
            .worker
            .take()
            .ok_or_else(|| QueueError("closed".into()))?;
        let result =
            match tokio::time::timeout(Duration::from_millis(timeout_ms), &mut worker).await {
                Ok(joined) => joined.map_err(error).and_then(|r| r),
                Err(_) => {
                    worker.abort();
                    let _ = worker.await;
                    Ok(())
                }
            };
        let path = self.path.clone();
        let queue = self.queue_id.clone();
        let ended = now()?;
        tokio::task::spawn_blocking(move || {
            Store::open(&path)?.interrupt_simulations(&queue, ended)
        })
        .await
        .map_err(error)?
        .map_err(error)?;
        result
    }
}
async fn dispatch(
    source: Arc<RpcSource>,
    path: PathBuf,
    options: QueueOptions,
    mut receiver: mpsc::Receiver<Pending>,
) -> Result<(), QueueError> {
    let mut active = JoinSet::new();
    loop {
        while active.len() >= options.concurrency {
            active
                .join_next()
                .await
                .expect("active set nonempty")
                .map_err(error)??;
        }
        let Some(pending) = receiver.recv().await else {
            break;
        };
        active.spawn(run_job(
            source.clone(),
            path.clone(),
            options.clone(),
            pending,
        ));
    }
    while let Some(result) = active.join_next().await {
        result.map_err(error)??;
    }
    Ok(())
}
async fn run_job(
    source: Arc<RpcSource>,
    path: PathBuf,
    options: QueueOptions,
    mut pending: Pending,
) -> Result<(), QueueError> {
    pending.record.queue_wait_ns = Some(
        pending
            .queued
            .elapsed()
            .as_nanos()
            .try_into()
            .map_err(error)?,
    );
    if pending.queued.elapsed() > Duration::from_millis(options.queue_timeout_ms) {
        pending.record.phase = SimulationPhase::QueueExpired;
        pending.record.outcome = SimulationOutcome::Unavailable;
        pending.record.error = Some("simulation queue deadline exceeded".into());
        pending.record.ended_at_ms = Some(now()?);
        return save(path, pending.record).await;
    }
    pending.record.phase = SimulationPhase::Running;
    pending.record.started_at_ms = Some(now()?);
    save(path.clone(), pending.record.clone()).await?;
    let started = Instant::now();
    match tokio::time::timeout(
        Duration::from_millis(options.call_timeout_ms),
        source.simulate(&pending.record.request),
    )
    .await
    {
        Ok(Ok(result)) => {
            pending.record.outcome = result.outcome.clone();
            pending.record.result = Some(result);
        }
        Ok(Err(failure)) => {
            pending.record.outcome = failure.outcome();
            pending.record.error = Some(failure.to_string());
            pending.record.error_evidence = failure.evidence().to_vec();
        }
        Err(_) => {
            pending.record.outcome = SimulationOutcome::Unknown;
            pending.record.error =
                Some("simulation deadline exceeded; execution outcome unknown".into());
        }
    }
    pending.record.elapsed_ns = Some(started.elapsed().as_nanos().try_into().map_err(error)?);
    pending.record.ended_at_ms = Some(now()?);
    pending.record.phase = SimulationPhase::Finished;
    save(path, pending.record).await
}
