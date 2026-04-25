/// Describes how the port was resolved — included in both human and JSON output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortSource {
    Flag,
    PidFile,
    Config,
    Default,
}

impl PortSource {
    pub fn as_str(self) -> &'static str {
        match self {
            PortSource::Flag => "flag",
            PortSource::PidFile => "pid_file",
            PortSource::Config => "config",
            PortSource::Default => "default",
        }
    }
}

/// `gateway status` — query the running server and print a summary.
///
/// # Exit codes (returned as `Ok(i32)`)
/// - `0` — server is running and healthy (HTTP 200 from /status)
/// - `1` — server is not running, or returned non-200
/// - `Err(_)` — system error (network failure, DNS resolution failure) → caller exits 2
pub async fn run(
    port: u16,
    source: PortSource,
    json_output: bool,
    output_config: accelmars_gateway_core::OutputConfig,
) -> anyhow::Result<i32> {
    let url = format!("http://127.0.0.1:{port}/status");

    let resp = match reqwest::get(&url).await {
        Ok(r) => r,
        Err(e) => {
            // Connection-refused is the normal "server not running" case → exit 1.
            // Other errors (DNS, timeout, etc.) are system errors → exit 2 (via Err).
            if e.is_connect() || e.is_request() {
                emit_not_running(port, source, json_output)?;
                return Ok(1);
            }
            return Err(anyhow::anyhow!("network error checking port {port}: {e:#}"));
        }
    };

    if !resp.status().is_success() {
        if json_output {
            let obj = serde_json::json!({
                "running": false,
                "checked_port": port,
                "http_status": resp.status().as_u16(),
                "source": source.as_str()
            });
            println!("{}", serde_json::to_string_pretty(&obj)?);
        } else {
            eprintln!(
                "Gateway is not responding on port {port}. Check if it's running: gateway start"
            );
        }
        return Ok(1);
    }

    let data: serde_json::Value = resp.json().await?;

    if json_output {
        let obj = serde_json::json!({
            "running": true,
            "version": data["version"].as_str().unwrap_or("unknown"),
            "port": data["port"].as_u64().unwrap_or(port as u64),
            "uptime_seconds": data["uptime_seconds"].as_u64().unwrap_or(0),
            "mode": data["mode"].as_str().unwrap_or("unknown"),
            "concurrency": data["concurrency"],
            "providers": data["providers"]
        });
        println!("{}", serde_json::to_string_pretty(&obj)?);
        return Ok(0);
    }

    // Human-readable output
    let version = data["version"].as_str().unwrap_or("unknown");
    let port_val = data["port"].as_u64().unwrap_or(port as u64);
    let mode = data["mode"].as_str().unwrap_or("unknown");
    let uptime = data["uptime_seconds"].as_u64().unwrap_or(0);
    let active = data["concurrency"]["active"].as_u64().unwrap_or(0);
    let max = data["concurrency"]["max"].as_u64().unwrap_or(0);

    println!("AccelMars Gateway v{version}");
    println!();
    println!(
        "Server:       running (port {port_val}, uptime {})",
        format_uptime(uptime)
    );
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

            let icon = if avail {
                output_config.colorize("✓", "\x1b[32m", "\x1b[0m")
            } else {
                output_config.colorize("✗", "\x1b[31m", "\x1b[0m")
            };
            let tag_str = if tags.is_empty() {
                String::new()
            } else {
                format!(" ({})", tags.join(", "))
            };
            let avail_str = if avail {
                output_config.colorize("available", "\x1b[32m", "\x1b[0m")
            } else {
                output_config.colorize("unavailable", "\x1b[31m", "\x1b[0m")
            };
            println!("  {icon} {name:<24}{avail_str}{tag_str}");
        }

        println!();
        println!("  Total: {available_count}/{total} available");
    }

    println!();
    println!("Run 'gateway stats' for usage or 'gateway complete \"<prompt>\"' to test.");

    Ok(0)
}

fn emit_not_running(port: u16, source: PortSource, json_output: bool) -> anyhow::Result<()> {
    if json_output {
        let obj = serde_json::json!({
            "running": false,
            "checked_port": port,
            "source": source.as_str()
        });
        println!("{}", serde_json::to_string_pretty(&obj)?);
    } else {
        eprintln!("Gateway is not responding on port {port}. Check if it's running: gateway start");
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
