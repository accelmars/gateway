//! `gateway keys` subcommands — create, list, revoke.

use crate::auth::AuthStore;

/// Create a new API key and print it once to stdout.
pub fn create(store: &AuthStore, name: &str) {
    match store.create_key(name) {
        Ok((full_key, _record)) => {
            // U5: raw key → stdout (capturable); advisory → stderr (human-readable)
            println!("{full_key}");
            eprintln!("Created API key: {full_key}");
            eprintln!("Store this key securely — it will not be shown again.");
        }
        Err(e) => {
            eprintln!("error: failed to create key: {e:#}");
            std::process::exit(1);
        }
    }
}

/// List all API keys (human table or JSON).
pub fn list(store: &AuthStore, json: bool, output_config: accelmars_gateway_core::OutputConfig) {
    let keys = match store.list_keys() {
        Ok(k) => k,
        Err(e) => {
            eprintln!("error: failed to list keys: {e:#}");
            std::process::exit(1);
        }
    };

    if json {
        let records: Vec<serde_json::Value> = keys
            .iter()
            .map(|k| {
                serde_json::json!({
                    "prefix": k.prefix.as_str(),
                    "name": k.name,
                    "created_at": k.created_at,
                    "last_used": k.last_used,
                    "active": k.active
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&records).unwrap_or_default()
        );
    } else {
        // C3: zero-result message with next step
        if keys.is_empty() {
            println!("No API keys found. Create one with: gateway keys create --name \"My App\"");
            return;
        }
        println!(
            "{:<14}  {:<20}  {:<22}  {:<22}  Status",
            "Prefix", "Name", "Created", "Last Used"
        );
        println!("{}", "-".repeat(90));
        for k in &keys {
            let last_used = k.last_used.as_deref().unwrap_or("-");
            // C4: colorize status column
            let status_str = if k.active { "active" } else { "revoked" };
            let status = if k.active {
                output_config.colorize(status_str, "\x1b[32m", "\x1b[0m")
            } else {
                output_config.colorize(status_str, "\x1b[2m", "\x1b[0m")
            };
            println!(
                "{:<14}  {:<20}  {:<22}  {:<22}  {}",
                k.prefix.as_str(),
                truncate(&k.name, 20),
                &k.created_at,
                last_used,
                status,
            );
        }
    }
}

/// Revoke a key by prefix.
///
/// # Exit codes
/// - `0` — revoked successfully (or dry-run preview shown)
/// - `1` — key not found (user error)
/// - `2` — auth store DB error (system error)
pub fn revoke(store: &AuthStore, prefix: &str, dry_run: bool, yes: bool) -> i32 {
    // Look up all keys to find the target and gather candidates for fuzzy suggestion
    let all_keys = match store.list_keys() {
        Ok(k) => k,
        Err(e) => {
            eprintln!("Error: Failed to look up key — {e:#}");
            return 2;
        }
    };

    // Find the specific key by prefix (active only — can't revoke an already-revoked key)
    let key = match all_keys
        .iter()
        .find(|k| k.prefix.as_str() == prefix && k.active)
    {
        Some(k) => k,
        None => {
            // C1: fuzzy suggestion using active key prefixes
            let active_prefixes: Vec<&str> = all_keys
                .iter()
                .filter(|k| k.active)
                .map(|k| k.prefix.as_str())
                .collect();
            let suggestions =
                accelmars_gateway_core::suggest_similar(prefix, &active_prefixes, 0.6);
            eprintln!(
                "Error: Key '{}' not found. Run 'gateway keys list' to see available keys.",
                prefix
            );
            if let Some(suggestion) = suggestions.first() {
                eprintln!("Did you mean: gateway keys revoke {}?", suggestion);
            }
            return 1; // exit 1 = user error (C9)
        }
    };

    // C7: --dry-run preview
    if dry_run {
        eprintln!("Would revoke: {} ({}).", key.name, prefix);
        return 0;
    }

    // C7: confirmation prompt unless --yes
    if !yes {
        eprint!(
            "Revoke {} ({})? This cannot be undone. [y/N] ",
            key.name, prefix
        );
        let mut input = String::new();
        std::io::stdin().read_line(&mut input).unwrap_or_default();
        if !input.trim().eq_ignore_ascii_case("y") {
            eprintln!("Aborted.");
            return 0;
        }
    }

    // Revoke
    match store.revoke_key(prefix) {
        Ok(true) => {
            // U3: next step on success
            eprintln!(
                "Revoked: {} ({}). Run 'gateway keys list' to see remaining active keys.",
                key.name, prefix
            );
            0
        }
        Ok(false) => {
            eprintln!("Error: Key '{}' not found.", prefix);
            1
        }
        Err(e) => {
            eprintln!("Error: Failed to revoke key — {e:#}");
            2 // exit 2 = system error (C9)
        }
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max.saturating_sub(3)])
    }
}
