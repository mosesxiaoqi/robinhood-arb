use crate::{
    config::{Config, parse_amount},
    pipeline::Pipeline,
    recovery::RecoveryPlan,
    simulation_queue::{QueueOptions, SimulationQueue},
};

use arb_adapters::{
    discovery::discover,
    rpc::{RpcOptions, RpcSource},
    store::Store,
};
use arb_core::{
    checkpoint::*,
    opportunity::CostEstimate,
    protocol::Bootstrap,
    research::{Candidate, RunSpec},
    simulation::*,
    state::State,
    types::*,
};
use serde::Serialize;
use std::{
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
fn error(e: impl std::fmt::Display) -> String {
    e.to_string()
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}
#[derive(Clone, Debug)]
pub struct DiskBudget {
    pub(crate) write_gate: Arc<std::sync::Mutex<()>>,
    pub path: PathBuf,
    pub limit: u64,
    pub reserve: u64,
}
impl DiskBudget {
    pub fn used(&self) -> Result<u64, String> {
        let mut bytes = 0u64;
        for suffix in ["", "-wal", "-shm"] {
            let mut name = self.path.as_os_str().to_os_string();
            name.push(suffix);
            match fs::metadata(PathBuf::from(name)) {
                Ok(m) => bytes = bytes.checked_add(m.len()).ok_or("disk size overflow")?,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(error(e)),
            }
        }
        Ok(bytes)
    }
    pub fn allows(&self, incoming: u64) -> Result<bool, String> {
        // SQLite may keep both DB pages and WAL frames; reserve covers indexes and stop metadata.
        let needed = incoming
            .checked_mul(3)
            .and_then(|n| n.checked_add(self.reserve))
            .and_then(|n| n.checked_add(1048576))
            .ok_or("disk reservation overflow")?;
        let parent = self
            .path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let free = fs2::available_space(parent).map_err(error)?;
        Ok(self
            .used()?
            .checked_add(needed)
            .is_some_and(|n| n <= self.limit)
            && free >= needed)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub enum RunStatus {
    Running,
    AwaitingPools,
    DataGapPaused,
    DiskPaused,
    Stopped,
    WindowEnded,
    Failed,
}
pub struct RuntimeEngine {
    store: Store,
    pub pipeline: Option<Pipeline>,
    pub budget: DiskBudget,
    pub run: RunSpec,
    path: PathBuf,
    _lock: File,
}
impl RuntimeEngine {
    pub fn open(config: &Config) -> Result<Self, String> {
        config.validate().map_err(error)?;
        if config.chain_id != 4663 {
            return Err("runtime network not verified".into());
        }
        let parent = config
            .database
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        fs::create_dir_all(parent).map_err(error)?;
        let mut lock_path = config.database.as_os_str().to_os_string();
        lock_path.push(".run.lock");
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(PathBuf::from(lock_path))
            .map_err(error)?;
        fs2::FileExt::try_lock_exclusive(&lock)
            .map_err(|_| "runtime already active for this database")?;
        let budget = DiskBudget {
            write_gate: Arc::new(std::sync::Mutex::new(())),
            path: config.database.clone(),
            limit: config.disk_budget_bytes,
            reserve: config.disk_reserve_bytes,
        };
        if !budget.allows(0)? {
            return Err("disk budget prevents runtime startup".into());
        }
        let mut store = Store::open(&config.database).map_err(error)?;
        let hash = config.research_hash();
        let run = RunSpec {
            run_id: config
                .run_id
                .clone()
                .unwrap_or_else(|| format!("live-{hash}")),
            config_hash: hash,
            algorithm_version: "pons-v2-v4-step1-v1".into(),
            registry_version: 1,
            quote_asset: config.quote_asset,
            amounts: config
                .amounts
                .iter()
                .map(|a| parse_amount(a).map_err(error))
                .collect::<Result<_, _>>()?,
            min_depth: parse_amount(&config.min_depth).map_err(error)?,
            min_profit: parse_amount(&config.min_profit).map_err(error)?,
            costs: CostEstimate {
                asset: config.quote_asset,
                amount: config
                    .additional_cost
                    .as_ref()
                    .map(|a| parse_amount(a).map_err(error))
                    .transpose()?,
                conversion: None,
                basis: if config.additional_cost.is_some() {
                    "explicit configured total additional cost estimate"
                } else {
                    "additional cost unavailable"
                }
                .into(),
            },
        };
        store.register_run(&run).map_err(error)?;
        let checkpoint = store.runtime_checkpoint(&run.run_id).map_err(error)?;
        let pipeline = checkpoint
            .map(|cp| {
                if cp.config_hash != run.config_hash
                    || cp.algorithm_version != run.algorithm_version
                {
                    return Err("checkpoint configuration mismatch".into());
                }
                Pipeline::new(
                    Store::open(&config.database).map_err(error)?,
                    cp.state,
                    run.clone(),
                )
                .map_err(error)
            })
            .transpose()?;
        let mut engine = Self {
            store,
            pipeline,
            budget,
            run,
            path: config.database.clone(),
            _lock: lock,
        };
        if let Some(id) = engine
            .store
            .pending_recovery(&engine.run.run_id)
            .map_err(error)?
        {
            let pipeline = engine
                .pipeline
                .as_mut()
                .ok_or("recovery requires checkpoint")?;
            RecoveryPlan::load(pipeline, id)
                .map_err(error)?
                .execute(pipeline)
                .map_err(error)?;
            engine.checkpoint()?;
        }
        Ok(engine)
    }
    pub fn next_collect(&self) -> Result<Option<u64>, String> {
        Ok(self
            .store
            .cursor(4663, "rpc")
            .map_err(error)?
            .map(|c| c.next_block))
    }
    pub fn pools(&self) -> Result<Vec<PoolDescriptor>, String> {
        let pools = self.store.read_pools(None, 129).map_err(error)?;
        if pools.len() > 128 {
            return Err("runtime pool universe exceeds 128; select a smaller database".into());
        }
        Ok(pools)
    }
    pub fn initialize(&mut self, bootstrap: Bootstrap) -> Result<(), String> {
        let gate = self.budget.write_gate.clone();
        let _guard = gate.lock().map_err(|_| "budget lock poisoned")?;
        if arb_core::route::two_leg_routes(
            &bootstrap
                .pools
                .iter()
                .map(|p| p.descriptor.clone())
                .collect::<Vec<_>>(),
            self.run.quote_asset,
        )
        .len()
        .saturating_mul(self.run.amounts.len())
            > 10000
        {
            return Err("route/amount sample budget exceeds 10000 per block".into());
        }

        if !self
            .budget
            .allows(serde_json::to_vec(&bootstrap).map_err(error)?.len() as u64)?
        {
            return Err("disk budget prevents bootstrap".into());
        }
        self.store.save_bootstrap(&bootstrap).map_err(error)?;
        self.pipeline = Some(
            Pipeline::new(
                Store::open(&self.path).map_err(error)?,
                State::from_bootstrap(bootstrap).map_err(error)?,
                self.run.clone(),
            )
            .map_err(error)?,
        );
        self.checkpoint()
    }
    fn checkpoint(&mut self) -> Result<(), String> {
        if let Some(p) = &self.pipeline {
            let checkpoint = Checkpoint {
                version: 1,
                research_run_id: Some(self.run.run_id.clone()),
                state: p.state.clone(),
                registry_version: self.run.registry_version,
                config_hash: self.run.config_hash,
                algorithm_version: self.run.algorithm_version.clone(),
                processing_cursor: ProcessingCursor {
                    last_raw_id: self.store.last_raw_id().map_err(error)?,
                    next_block: p
                        .view()
                        .position
                        .block_number
                        .checked_add(1)
                        .ok_or("block overflow")?,
                },
            };
            self.store
                .save_runtime_checkpoint(&checkpoint)
                .map_err(error)?;
        }
        Ok(())
    }
    pub fn commit(&mut self, records: &[RawRecord], stopped: bool) -> Result<RunStatus, String> {
        let gate = self.budget.write_gate.clone();
        let _guard = gate.lock().map_err(|_| "budget lock poisoned")?;
        if stopped {
            return Ok(RunStatus::Stopped);
        }
        let size = serde_json::to_vec(records).map_err(error)?.len() as u64;
        if !self.budget.allows(size)? {
            return Ok(RunStatus::DiskPaused);
        }
        let raw = records
            .iter()
            .find(|r| r.kind == "block")
            .ok_or("missing block")?;
        let at = raw.position.as_ref().ok_or("missing position")?;
        let value: serde_json::Value = serde_json::from_slice(&raw.payload).map_err(error)?;
        if let Some(cursor) = self.store.cursor(4663, "rpc").map_err(error)?
            && (cursor.next_block != at.block_number
                || cursor
                    .last_block_hash
                    .is_some_and(|h| value["result"]["parentHash"] != serde_json::json!(h)))
        {
            self.store
                .record_gap(
                    4663,
                    "rpc",
                    at.block_number,
                    "parent changed; bounded recovery required",
                )
                .map_err(error)?;
            return Ok(RunStatus::DataGapPaused);
        }
        let pools = discover(records).map_err(error)?;
        self.store
            .append_raw_with_pools(
                records,
                &SourceCursor {
                    chain_id: 4663,
                    source: "rpc".into(),
                    next_block: at.block_number.checked_add(1).ok_or("block overflow")?,
                    last_block_hash: Some(at.block_hash),
                },
                &pools,
            )
            .map_err(error)?;
        if let Some(p) = &mut self.pipeline {
            p.process_records(records, now()).map_err(error)?;
            self.checkpoint()?;
            Ok(RunStatus::Running)
        } else {
            Ok(RunStatus::AwaitingPools)
        }
    }
    pub fn replay_committed(&mut self) -> Result<bool, String> {
        let gate = self.budget.write_gate.clone();
        let _guard = gate.lock().map_err(|_| "budget lock poisoned")?;
        if let Some(p) = &mut self.pipeline
            && let Some(next) = self
                .store
                .cursor(4663, "rpc")
                .map_err(error)?
                .map(|c| c.next_block)
            && p.view().position.block_number + 1 < next
        {
            if !self.budget.allows(1048576)? {
                return Err("disk budget prevents replay".into());
            }
            let number = p.view().position.block_number + 1;
            crate::replay::replay_chain(p, number).map_err(error)?;
            self.checkpoint()?;
            return Ok(true);
        }
        Ok(false)
    }
    /// Backfill at most 128 blocks; deeper or bootstrap-crossing forks remain explicitly paused.
    pub async fn recover(
        &mut self,
        source: &RpcSource,
        incoming: Vec<RawRecord>,
    ) -> Result<usize, String> {
        let pipeline = self
            .pipeline
            .as_ref()
            .ok_or("reorg before bootstrap; verified reinitialization required")?;
        let descriptors = pipeline
            .view()
            .pools
            .into_iter()
            .map(|p| p.descriptor)
            .collect::<Vec<_>>();
        let mut old = pipeline.view().position;
        let mut replacements = vec![incoming];
        let mut bytes = 0usize;
        let mut common = false;
        for _ in 0..128 {
            let records = source.fetch_block(old.block_number).await.map_err(error)?;
            let at = records[0].position.as_ref().ok_or("recovery position")?;
            if at.block_hash == old.block_hash {
                common = true;
                break;
            }
            bytes = bytes
                .checked_add(serde_json::to_vec(&records).map_err(error)?.len())
                .ok_or("recovery size")?;
            if bytes > 64 * 1024 * 1024 {
                return Err("recovery byte budget".into());
            }
            replacements.push(records);
            let prior = pipeline
                .store()
                .find_derived(&self.run.run_id, old.block_hash)
                .map_err(error)?
                .ok_or("fork crosses initial snapshot; new verified bootstrap required")?;
            old = ChainPosition {
                block_number: old.block_number.checked_sub(1).ok_or("recovery genesis")?,
                block_hash: prior.batch.parent_hash,
                offset: Offset::BlockEnd,
            };
        }
        if !common {
            return Err("recovery exceeds 128-block runtime lookback".into());
        }
        replacements.reverse();
        let batches = replacements
            .iter()
            .map(|r| arb_adapters::assemble::assemble_block(r, &descriptors).map_err(error))
            .collect::<Result<Vec<_>, _>>()?;
        let plan = RecoveryPlan::build(pipeline, batches).map_err(error)?;
        let gate = self.budget.write_gate.clone();
        let _guard = gate.lock().map_err(|_| "budget lock poisoned")?;
        if !self.budget.allows(bytes as u64 + 1048576)? {
            return Err("disk budget prevents recovery".into());
        }
        let cursor = self
            .store
            .cursor(4663, "rpc")
            .map_err(error)?
            .ok_or("recovery cursor missing")?;
        for records in &replacements {
            self.store
                .append_raw_with_pools(records, &cursor, &discover(records).map_err(error)?)
                .map_err(error)?;
        }
        let pipeline = self.pipeline.as_mut().ok_or("recovery pipeline missing")?;
        plan.execute(pipeline).map_err(error)?;
        Ok(replacements.len())
    }
    fn candidates(&self) -> Result<Vec<Candidate>, String> {
        match &self.pipeline {
            Some(p) => Ok(p
                .store()
                .find_derived(&self.run.run_id, p.view().position.block_hash)
                .map_err(error)?
                .map_or(vec![], |b| b.candidates)),
            None => Ok(vec![]),
        }
    }
}
#[derive(Debug, Serialize)]
pub struct RunSummary {
    pub run_id: String,
    pub status: RunStatus,
    pub started_at_ms: u64,
    pub ended_at_ms: u64,
    pub collected_blocks: u64,
    pub processed_blocks: u64,
    pub first_collected: Option<u64>,
    pub last_collected: Option<u64>,
    pub disk_start: u64,
    pub disk_end: u64,
    pub max_processing_ns: u64,
    pub error: Option<String>,
}
pub async fn run(
    config: Config,
    mut stop: tokio::sync::watch::Receiver<bool>,
) -> Result<RunSummary, String> {
    let mut engine = RuntimeEngine::open(&config)?;
    let stamp = now();
    let source = Arc::new(
        RpcSource::new(
            &config.rpc_url,
            RpcOptions {
                chain_id: 4663,
                source: "rpc".into(),
                run_id: format!("runtime-{}-{stamp}", std::process::id()),
                requests_per_second: config.requests_per_second,
                max_concurrency: config.max_concurrency,
                retry_limit: config.retry_limit,
                timeout_ms: config.timeout_ms,
                max_response_bytes: config.max_response_bytes,
            },
        )
        .map_err(error)?,
    );
    let queue_id = format!(
        "sim-{}",
        alloy_primitives::keccak256(engine.run.run_id.as_bytes())
    );
    engine
        .store
        .interrupt_simulations(&queue_id, stamp)
        .map_err(error)?;
    let queue = SimulationQueue::new(
        Arc::new(source.simulation_lane()),
        config.database.clone(),
        QueueOptions {
            queue_id,
            capacity: config.queue_capacity,
            concurrency: config.max_concurrency.min(64),
            queue_timeout_ms: config.timeout_ms,
            call_timeout_ms: config.timeout_ms,
            disk_budget: Some(engine.budget.clone()),
        },
    )
    .map_err(error)?;
    let mut summary = RunSummary {
        run_id: engine.run.run_id.clone(),
        status: RunStatus::Running,
        started_at_ms: stamp,
        ended_at_ms: stamp,
        collected_blocks: 0,
        processed_blocks: 0,
        first_collected: None,
        last_collected: None,
        disk_start: engine.budget.used()?,
        disk_end: 0,
        max_processing_ns: 0,
        error: None,
    };
    let deadline = tokio::time::Instant::now() + Duration::from_secs(config.window_seconds);
    let work_stop = stop.clone();
    let work = async {
        loop {
            if *work_stop.borrow() {
                summary.status = RunStatus::Stopped;
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                summary.status = RunStatus::WindowEnded;
                break;
            }
            if !engine
                .budget
                .allows(config.max_response_bytes as u64 * 18)?
            {
                summary.status = RunStatus::DiskPaused;
                break;
            }
            if engine.replay_committed()? {
                summary.processed_blocks += 1;
                continue;
            }
            let latest = source.latest_position().await.map_err(error)?.block_number;
            let safe = latest.saturating_sub(config.confirmations);
            let next = engine
                .next_collect()?
                .or(config.start_block)
                .unwrap_or(safe);
            if next > safe {
                tokio::time::sleep(Duration::from_millis(250)).await;
                continue;
            }
            let records = source.fetch_block(next).await.map_err(error)?;
            if *work_stop.borrow() || tokio::time::Instant::now() >= deadline {
                continue;
            }
            let started = Instant::now();
            let status = engine.commit(&records, false)?;
            if status == RunStatus::DiskPaused {
                summary.status = status;
                break;
            }
            if status == RunStatus::DataGapPaused {
                match engine.recover(&source, records.clone()).await {
                    Ok(count) => {
                        summary.processed_blocks += count as u64;
                    }
                    Err(reason) => {
                        summary.status = RunStatus::DataGapPaused;
                        summary.error = Some(reason);
                        break;
                    }
                }
            }
            if status == RunStatus::Running {
                summary.processed_blocks += 1;
            }
            summary.collected_blocks += 1;
            summary.first_collected.get_or_insert(next);
            summary.last_collected = Some(next);
            if engine.pipeline.is_none()
                || engine.pipeline.as_ref().is_some_and(|p| {
                    p.view()
                        .pools
                        .iter()
                        .any(|p| p.descriptor.verification == PoolVerification::Pending)
                })
            {
                let pools = match &engine.pipeline {
                    Some(p) => p.view().pools.into_iter().map(|p| p.descriptor).collect(),
                    None => engine.pools()?,
                };
                if !pools.is_empty() {
                    let at = records[0]
                        .position
                        .as_ref()
                        .ok_or("missing bootstrap position")?;
                    let bootstrap = source.bootstrap(&pools, at.clone()).await.map_err(error)?;
                    engine.initialize(bootstrap)?;
                }
            }
            summary.max_processing_ns = summary
                .max_processing_ns
                .max(started.elapsed().as_nanos().try_into().unwrap_or(u64::MAX));
            summary.status = if engine.pipeline.is_some() {
                RunStatus::Running
            } else {
                RunStatus::AwaitingPools
            };
            for candidate in engine.candidates()? {
                let view = engine
                    .pipeline
                    .as_ref()
                    .ok_or("missing candidate view")?
                    .view();
                let pools = candidate
                    .opportunity
                    .route
                    .legs
                    .iter()
                    .map(|leg| {
                        view.pools
                            .iter()
                            .find(|p| p.descriptor.id == leg.pool)
                            .map(|p| SimulationPool {
                                currency0: p.descriptor.currency0,
                                currency1: p.descriptor.currency1,
                                fee: p.descriptor.lp_fee,
                                tick_spacing: p.descriptor.tick_spacing,
                                hook: p.descriptor.hook,
                            })
                    })
                    .collect::<Option<Vec<_>>>()
                    .ok_or("candidate pool missing")?;
                let request = SimulationRequest {
                    position: candidate.opportunity.position.clone(),
                    route: candidate.opportunity.route.clone(),
                    pools: pools
                        .try_into()
                        .map_err(|_| "simulation requires two legs")?,
                    amount: candidate.opportunity.amount_in,
                    funding_balance: candidate.opportunity.amount_in,
                    fail_second: false,
                };
                eprintln!(
                    "run={} source=rpc block={} candidate={} stage=simulation-submit",
                    summary.run_id, next, candidate.id
                );
                queue.submit(candidate, request).await.map_err(error)?;
            }
            eprintln!(
                "run={} source=rpc block={} status={:?} processing_ns={} queue={}",
                summary.run_id,
                next,
                summary.status,
                summary.max_processing_ns,
                queue.queued_len()
            );
        }
        Ok::<(), String>(())
    };
    let result = tokio::select! {
           result=work=>result,
           _=async {loop {if *stop.borrow() {break;}
    if stop.changed().await.is_err() {break;}}}=>{summary.status=RunStatus::Stopped;Ok(())},
           _=tokio::time::sleep_until(deadline)=>{summary.status=RunStatus::WindowEnded;Ok(())},
       };
    if let Err(e) = result {
        summary.status = RunStatus::Failed;
        summary.error = Some(e);
    }
    if let Err(e) = queue.shutdown(config.timeout_ms.min(5000)).await {
        summary.status = RunStatus::Failed;
        summary.error = Some(e.to_string());
    }
    summary.ended_at_ms = now();
    summary.disk_end = engine.budget.used()?;
    engine
        .store
        .save_runtime_status(
            &summary.run_id,
            &serde_json::to_value(&summary).map_err(error)?,
        )
        .map_err(error)?;
    Ok(summary)
}
