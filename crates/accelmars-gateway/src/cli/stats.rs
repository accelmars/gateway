use crate::cost::CostTracker;

/// `gateway stats` — query the SQLite cost DB and print a usage summary.
pub fn run(
    since: Option<&str>,
    json_output: bool,
    provider_filter: Option<&str>,
    tier_filter: Option<&str>,
) -> anyhow::Result<()> {
    let db_path = CostTracker::default_path();

    if !db_path.exists() {
        if json_output {
            println!(
                "{{\"total_calls\":0,\"total_cost_usd\":0.0,\"by_provider\":[],\"by_tier\":[]}}"
            );
        } else {
            println!("AccelMars Gateway — Usage Summary");
            println!();
            println!("No data yet. Run `gateway serve` and make some requests first.");
            println!("DB path: {}", db_path.display());
        }
        return Ok(());
    }

    let tracker = CostTracker::open(&db_path)?;
    let summary = tracker.summary(since)?;

    // Apply optional filters
    let by_provider: Vec<_> = summary
        .by_provider
        .iter()
        .filter(|p| provider_filter.is_none_or(|f| p.provider == f))
        .collect();

    let by_tier: Vec<_> = summary
        .by_tier
        .iter()
        .filter(|t| tier_filter.is_none_or(|f| t.tier == f))
        .collect();

    // Determine if filters were applied and produced empty results
    let filters_applied = provider_filter.is_some() || tier_filter.is_some();
    let no_matching_records =
        filters_applied && by_provider.is_empty() && by_tier.is_empty() && summary.total_calls > 0;

    if json_output {
        if no_matching_records {
            let mut filters = serde_json::Map::new();
            if let Some(p) = provider_filter {
                filters.insert("provider".to_string(), serde_json::json!(p));
            }
            if let Some(t) = tier_filter {
                filters.insert("tier".to_string(), serde_json::json!(t));
            }
            let obj = serde_json::json!({
                "total_calls": 0,
                "filters_applied": filters,
                "note": "no matching records"
            });
            println!("{}", serde_json::to_string_pretty(&obj)?);
            return Ok(());
        }

        let obj = serde_json::json!({
            "period": since.unwrap_or("all-time"),
            "total_calls": summary.total_calls,
            "total_cost_usd": summary.total_cost_usd,
            "by_provider": by_provider.iter().map(|p| serde_json::json!({
                "provider": p.provider,
                "calls": p.calls,
                "cost_usd": p.cost_usd,
            })).collect::<Vec<_>>(),
            "by_tier": by_tier.iter().map(|t| serde_json::json!({
                "tier": t.tier,
                "calls": t.calls,
                "cost_usd": t.cost_usd,
            })).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&obj)?);
        return Ok(());
    }

    // Human-readable output
    println!("AccelMars Gateway — Usage Summary");
    println!();

    if no_matching_records {
        let filter_desc = match (provider_filter, tier_filter) {
            (Some(p), Some(t)) => format!("provider={p}, tier={t}"),
            (Some(p), None) => format!("provider={p}"),
            (None, Some(t)) => format!("tier={t}"),
            (None, None) => unreachable!(),
        };
        println!("No matching records. (filters: {filter_desc})");
        return Ok(());
    }

    let period = since.unwrap_or("all-time");
    println!("Period: {period}");
    println!();
    println!("  Total calls:  {}", summary.total_calls);
    println!("  Total cost:   ${:.4}", summary.total_cost_usd);
    println!();

    if !by_provider.is_empty() {
        println!("  By provider:");
        for p in &by_provider {
            let cost_str = if p.cost_usd == 0.0 {
                "$0.00".to_string()
            } else {
                format!("${:.4}", p.cost_usd)
            };
            println!(
                "    {:<24} {:>6} calls    {}",
                p.provider, p.calls, cost_str
            );
        }
        println!();
    }

    if !by_tier.is_empty() {
        println!("  By tier:");
        for t in &by_tier {
            let cost_str = if t.cost_usd == 0.0 {
                "$0.00".to_string()
            } else {
                format!("${:.4}", t.cost_usd)
            };
            println!("    {:<24} {:>6} calls    {}", t.tier, t.calls, cost_str);
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Unit tests — stats output
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Run stats with a nonexistent filter against an empty DB path (no DB file).
    /// Expected: no output, no panic.
    #[test]
    fn stats_with_no_db_returns_ok() {
        // Temporarily point to a nonexistent DB path
        std::env::set_var(
            "GATEWAY_DB_PATH",
            "/tmp/.test-gateway-stats-nonexistent-99999.db",
        );
        let result = run(None, false, None, None);
        std::env::remove_var("GATEWAY_DB_PATH");
        assert!(
            result.is_ok(),
            "stats with no DB should return Ok: {result:?}"
        );
    }
}
