/// `gateway status` — query the running server and print a human-readable summary.
pub async fn run(port: u16) -> anyhow::Result<()> {
    let url = format!("http://127.0.0.1:{port}/status");

    let resp = match reqwest::get(&url).await {
        Ok(r) => r,
        Err(_) => {
            println!("AccelMars Gateway");
            println!();
            println!("Server:      not running (nothing listening on port {port})");
            println!("Tip: start with `gateway serve`");
            return Ok(());
        }
    };

    if !resp.status().is_success() {
        anyhow::bail!("server returned HTTP {}", resp.status());
    }

    let data: serde_json::Value = resp.json().await?;

    let version = data["version"].as_str().unwrap_or("unknown");
    let port_val = data["port"].as_u64().unwrap_or(port as u64);
    let mode = data["mode"].as_str().unwrap_or("unknown");
    let uptime = data["uptime_seconds"].as_u64().unwrap_or(0);
    let active = data["concurrency"]["active"].as_u64().unwrap_or(0);
    let max = data["concurrency"]["max"].as_u64().unwrap_or(0);

    let uptime_str = format_uptime(uptime);

    println!("AccelMars Gateway v{version}");
    println!();
    println!("Server:       running (port {port_val}, uptime {uptime_str})");
    println!("Mode:         {mode}");
    println!("Concurrency:  {active}/{max} active");
    println!();
    println!("Providers:");

    if let Some(providers) = data["providers"].as_array() {
        let total = providers.len();
        let available_count = providers
            .iter()
            .filter(|p| p["available"].as_bool().unwrap_or(false))
            .count();

        for p in providers {
            let name = p["name"].as_str().unwrap_or("?");
            let avail = p["available"].as_bool().unwrap_or(false);
            let tags: Vec<&str> = p["tags"]
                .as_array()
                .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
                .unwrap_or_default();

            let status_icon = if avail { "✓" } else { "✗" };
            let tag_str = if tags.is_empty() {
                String::new()
            } else {
                format!(" ({})", tags.join(", "))
            };
            let avail_str = if avail { "available" } else { "unavailable" };
            println!("  {status_icon} {name:<24}{avail_str}{tag_str}");
        }

        println!();
        println!("  Total: {available_count}/{total} available");
    }

    Ok(())
}

fn format_uptime(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else {
        format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
    }
}
