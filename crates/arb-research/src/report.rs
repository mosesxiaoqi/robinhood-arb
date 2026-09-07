use crate::{opportunities::OpportunityAnalysis, wallets::attribute};
use arb_core::{
    research::{DerivedBlock, RunSpec},
    simulation::SimulationRecord,
    wallet::WalletFacts,
};
use std::{
    collections::BTreeMap,
    io::{self, Write},
};
/// RFC 4180 fields; quote every field, including embedded CR/LF and quotes.
pub fn csv_row(out: &mut impl Write, fields: &[String]) -> io::Result<()> {
    for (i, field) in fields.iter().enumerate() {
        if i > 0 {
            write!(out, ",")?;
        }
        write!(out, "\"{}\"", field.replace('"', "\"\""))?;
    }
    writeln!(out, "\r")
}
fn safe(text: &str) -> String {
    text.replace(['\r', '\n'], " ")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
#[allow(clippy::too_many_arguments)]
pub fn write_report(
    markdown: &mut impl Write,
    csv: &mut impl Write,
    run: &RunSpec,
    analysis: &OpportunityAnalysis,
    blocks: &[DerivedBlock],
    simulations: &[SimulationRecord],
    wallets: &[WalletFacts],
    gaps: (u64, u64),
) -> io::Result<()> {
    writeln!(
        markdown,
        "# 只读模拟，非真实成交\n\n运行：{}\n\n覆盖区间：{}–{}；完整块 {}；数据缺口 {}；孤块 {}。\n\nRPC 未解决缺口记录 {}；本库该网络 Feed 历史观测缺口区间 {}（非运行窗口计数）。\n",
        safe(&run.run_id),
        analysis.from,
        analysis.to,
        analysis.complete_blocks,
        analysis.gap_blocks,
        analysis.orphan_blocks,
        gaps.0,
        gaps.1
    )?;
    writeln!(
        markdown,
        "配置摘要：{}；算法：{}；注册表版本：{}。\n\n报价资产：{}；采样金额：{}；最小深度：{}；净利润阈值：{}。\n\n额外成本：{}；依据：{}。费用未知的候选不算通过。\n",
        run.config_hash,
        safe(&run.algorithm_version),
        run.registry_version,
        run.quote_asset,
        run.amounts
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", "),
        run.min_depth,
        run.min_profit,
        run.costs
            .amount
            .map(|v| v.to_string())
            .unwrap_or("未知".into()),
        safe(&run.costs.basis)
    )?;
    writeln!(
        markdown,
        "## 机会与容量\n\n机会区间 {}；代币×完整块样本 {}（含失败币）；排除池×块 {}。容量仅为最大已测试有效金额，不做连续容量插值，不跨报价资产相加。模拟容量沿用候选成本估计，不是实际成交收益。\n",
        analysis.windows.len(),
        analysis.token_block_samples,
        analysis.excluded_pool_samples
    )?;
    csv_row(
        csv,
        &[
            "run".into(),
            "kind".into(),
            "from".into(),
            "to".into(),
            "quote_asset".into(),
            "sampled_capacity".into(),
            "simulated_capacity".into(),
            "detail".into(),
        ],
    )?;
    csv_row(
        csv,
        &[
            run.run_id.clone(),
            "coverage".into(),
            analysis.from.to_string(),
            analysis.to.to_string(),
            run.quote_asset.to_string(),
            "".into(),
            "unknown".into(),
            format!(
                "complete={},gap={},token_block_samples={}",
                analysis.complete_blocks, analysis.gap_blocks, analysis.token_block_samples
            ),
        ],
    )?;
    for window in &analysis.windows {
        let verified = window
            .simulated_capacity
            .map(|v| v.to_string())
            .unwrap_or("未知".into());
        let route = serde_json::to_string(&window.route).map_err(io::Error::other)?;
        writeln!(
            markdown,
            "- {}–{}，网络 {}，报价资产 {}：报价容量 {}；模拟验证容量 {}。路线 `{}`",
            window.start.block_number,
            window.end.block_number,
            window.chain_id,
            window.quote_asset,
            window.sampled_capacity,
            verified,
            safe(&route)
        )?;
        csv_row(
            csv,
            &[
                window.run_id.clone(),
                "opportunity".into(),
                window.start.block_number.to_string(),
                window.end.block_number.to_string(),
                window.quote_asset.to_string(),
                window.sampled_capacity.to_string(),
                verified,
                route,
            ],
        )?;
    }
    writeln!(markdown, "\n排除原因（金额样本计数）：")?;
    for (reason, count) in &analysis.exclusions {
        writeln!(markdown, "- {}：{}", safe(reason), count)?;
    }
    let mut statuses = BTreeMap::new();
    for s in simulations {
        *statuses
            .entry(format!(
                "{:?}/{:?}/canonical={}/validates={}",
                s.phase, s.outcome, s.canonical, s.validates_original_candidate
            ))
            .or_insert(0u64) += 1;
    }
    writeln!(
        markdown,
        "\n## 模拟\n\n无结果、队列拒绝、不可用与超时都不是模拟成功；晚到孤块结果仅存档。"
    )?;
    if statuses.is_empty() {
        writeln!(markdown, "\n未模拟：0 条结果；能力未知。")?;
    }
    for (status, count) in statuses {
        writeln!(markdown, "- {}：{}", status, count)?;
        csv_row(
            csv,
            &[
                run.run_id.clone(),
                "simulation".into(),
                "".into(),
                "".into(),
                run.quote_asset.to_string(),
                "".into(),
                "".into(),
                format!("{status}: {count}"),
            ],
        )?;
    }
    for s in simulations {
        csv_row(
            csv,
            &[
                run.run_id.clone(),
                "simulation-detail".into(),
                s.expected_position.block_number.to_string(),
                s.expected_position.block_number.to_string(),
                run.quote_asset.to_string(),
                s.request.amount.to_string(),
                s.validates_original_candidate.to_string(),
                format!(
                    "candidate={},view={},phase={:?},outcome={:?},queued={:?},started={:?},ended={:?},wait_ns={:?},elapsed_ns={:?},canonical={}",
                    s.candidate_id,
                    s.view_id,
                    s.phase,
                    s.outcome,
                    s.queued_at_ms,
                    s.started_at_ms,
                    s.ended_at_ms,
                    s.queue_wait_ns,
                    s.elapsed_ns,
                    s.canonical
                ),
            ],
        )?;
    }
    let mut timings = BTreeMap::<String, (u64, u128, u64)>::new();
    for block in blocks.iter().filter(|b| b.canonical) {
        for t in &block.timings {
            let e = timings.entry(t.stage.clone()).or_default();
            e.0 += 1;
            e.1 += u128::from(t.elapsed_ns);
            e.2 = e.2.max(t.elapsed_ns);
        }
    }
    writeln!(
        markdown,
        "\n## 延迟与时间质量\n\n以下为本机单调时钟阶段耗时，不推断网络传播延迟；跨时钟域和回拨以原记录标记为准。"
    )?;
    for (stage, (count, total, max)) in timings {
        writeln!(
            markdown,
            "- {}：{} 次，平均 {} ns，最大 {} ns",
            safe(&stage),
            count,
            total / u128::from(count),
            max
        )?;
    }
    for b in blocks {
        if let Some(q) = &b.time_quality {
            writeln!(
                markdown,
                "- 块 {}：时钟回拨 {}；多时钟域 {}",
                b.view.position.block_number, q.clock_regressions, q.multiple_clock_domains
            )?;
        }
    }
    writeln!(
        markdown,
        "\n## 钱包证据\n\n{} 份事实。未采集或缺 trace/估值/入出金/成本基础时净利润未知；转账与合约收款角色不等于套利交易者。",
        wallets.len()
    )?;
    for facts in wallets {
        let a = attribute(facts);
        writeln!(
            markdown,
            "- 交易 {}，钱包 {}：{:?}；角色 {}；完整 {}；资产差范围 {:?}；已实现利润未知。",
            facts.transaction_hash,
            facts.wallet,
            a.activity,
            a.roles.join(", "),
            a.complete,
            facts.balance_scope
        )?;
        for c in &facts.changes {
            writeln!(
                markdown,
                "  - 资产 {}：{} → {}（原生单位）",
                c.asset, c.before, c.after
            )?;
        }
        csv_row(
            csv,
            &[
                run.run_id.clone(),
                "wallet".into(),
                facts.position.block_number.to_string(),
                facts.position.block_number.to_string(),
                "".into(),
                "".into(),
                "unknown".into(),
                format!(
                    "tx={},wallet={},activity={:?},complete={}",
                    facts.transaction_hash, facts.wallet, a.activity, a.complete
                ),
            ],
        )?;
    }
    Ok(())
}
