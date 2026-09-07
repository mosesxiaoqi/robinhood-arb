use arb_adapters::store::{ReportTable, Store};
use arb_core::{research::DerivedBlock, simulation::SimulationRecord, wallet::WalletFacts};
use std::{
    fs::{self, OpenOptions},
    io::{BufWriter, Write},
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};
fn error(e: impl std::fmt::Display) -> String {
    e.to_string()
}
pub fn export_report(
    database: &Path,
    run_id: &str,
    out: &Path,
    range: Option<(u64, u64)>,
) -> Result<(), String> {
    if out.exists() {
        return Err("output already exists; use a new report directory".into());
    }
    let store = Store::open_report(database).map_err(error)?;
    if store.has_pending_recovery(run_id).map_err(error)? {
        return Err("run recovery pending; report paused".into());
    }
    let run = store.load_run(run_id).map_err(error)?;
    let runtime = store.runtime_status(run_id).map_err(error)?;
    let observed_range = runtime.as_ref().and_then(|s| {
        Some((
            s["first_collected"].as_u64()?,
            s["last_collected"].as_u64()?,
        ))
    });
    let (from, to) = range
        .or(store.report_bounds(run_id).map_err(error)?)
        .or(observed_range)
        .unwrap_or((0, 0));
    if from > to || to - from >= 10000 {
        return Err("report window exceeds 10000 blocks; supply --from and --to".into());
    }
    let mut blocks = vec![];
    let mut simulations = vec![];
    let mut wallets = vec![];
    let mut total_bytes = 0usize;
    let mut total_rows = 0usize;
    for table in [
        ReportTable::Blocks,
        ReportTable::Simulations,
        ReportTable::Wallets,
    ] {
        let mut after = 0;
        loop {
            let page = store
                .read_report_page(table, run_id, after, from, to)
                .map_err(error)?;
            if page.is_empty() {
                break;
            }
            for (id, value) in page {
                after = id;
                total_rows += 1;
                if total_rows > 100000 {
                    return Err("report row scan budget".into());
                }
                let position = match table {
                    ReportTable::Blocks => &value["view"]["position"],
                    ReportTable::Simulations => &value["expected_position"],
                    ReportTable::Wallets => &value["position"],
                };
                let number = position["block_number"]
                    .as_u64()
                    .ok_or("report record position")?;
                if !(from..=to).contains(&number) {
                    continue;
                }
                total_bytes = total_bytes
                    .checked_add(serde_json::to_vec(&value).map_err(error)?.len())
                    .ok_or("report size overflow")?;
                if total_bytes > 64 * 1024 * 1024 {
                    return Err("report memory budget; use a smaller block window".into());
                }
                match table {
                    ReportTable::Blocks => {
                        blocks.push(serde_json::from_value::<DerivedBlock>(value).map_err(error)?)
                    }
                    ReportTable::Simulations => simulations
                        .push(serde_json::from_value::<SimulationRecord>(value).map_err(error)?),
                    ReportTable::Wallets => {
                        wallets.push(serde_json::from_value::<WalletFacts>(value).map_err(error)?)
                    }
                }
            }
        }
    }
    let canonical = blocks
        .iter()
        .filter(|b| b.canonical)
        .map(|b| b.view.position.block_hash)
        .collect::<std::collections::BTreeSet<_>>();
    wallets.retain(|w| canonical.contains(&w.position.block_hash));
    let analysis =
        arb_research::opportunities::summarize_opportunities(&blocks, &simulations, from, to)
            .map_err(error)?;
    let chain = blocks
        .first()
        .and_then(|b| b.view.pools.first())
        .map_or(4663, |p| p.descriptor.id.chain_id);
    let gaps = store.report_gap_counts(chain, from, to).map_err(error)?;
    let parent = out
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent).map_err(error)?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(error)?
        .as_nanos();
    let temporary = parent.join(format!(".arb-report-{}-{nonce}.tmp", std::process::id()));
    fs::create_dir(&temporary).map_err(error)?;
    let result = (|| {
        let mut markdown = BufWriter::new(
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(temporary.join("report.md"))
                .map_err(error)?,
        );
        let mut csv = BufWriter::new(
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(temporary.join("summary.csv"))
                .map_err(error)?,
        );
        arb_research::report::write_report(
            &mut markdown,
            &mut csv,
            &run,
            &analysis,
            &blocks,
            &simulations,
            &wallets,
            gaps,
        )
        .map_err(error)?;
        if let Some(runtime) = &runtime {
            writeln!(markdown,"\n## 最近运行窗口\n\n原始采集覆盖与完成状态分析分开计数；无池时仅采集原始记录，未建立分析视图。\n\n```json\n{}\n```",serde_json::to_string_pretty(runtime).map_err(error)?).map_err(error)?;
        }
        markdown.flush().map_err(error)?;
        csv.flush().map_err(error)?;
        markdown.get_ref().sync_all().map_err(error)?;
        csv.get_ref().sync_all().map_err(error)?;
        if out.exists() {
            return Err("output appeared during export".into());
        }
        fs::rename(&temporary, out).map_err(error)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&temporary);
    }
    result
}
