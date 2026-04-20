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

    if json_output {
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
    let period = since.unwrap_or("all-time");
    println!("Period: {period}");
    println!();
    println!("  Total calls:  {}", summary.total_calls);
    println!("  Total cost:   ${:.4}", summary.total_cost_usd);
    println!("  Cache hit rate: 0% (no cache yet)");
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
