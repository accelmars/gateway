//! `gateway keys` subcommands — create, list, revoke.

use crate::auth::AuthStore;

/// Create a new API key and print it once to stdout.
pub fn create(store: &AuthStore, name: &str) {
    match store.create_key(name) {
        Ok((full_key, _record)) => {
            println!("Created API key: {full_key}");
            println!("Store this key securely — it will not be shown again.");
        }
        Err(e) => {
            eprintln!("error: failed to create key: {e:#}");
            std::process::exit(1);
        }
    }
}

/// List all API keys (human table or JSON).
pub fn list(store: &AuthStore, json: bool) {
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
        if keys.is_empty() {
            println!("No API keys found. Create one with: gateway keys create --name <name>");
            return;
        }
        println!(
            "{:<14}  {:<20}  {:<22}  {:<22}  Status",
            "Prefix", "Name", "Created", "Last Used"
        );
        println!("{}", "-".repeat(90));
        for k in &keys {
            let last_used = k.last_used.as_deref().unwrap_or("-");
            let status = if k.active { "active" } else { "revoked" };
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

/// Revoke a key by prefix. Returns exit code (0 = revoked, 1 = not found).
pub fn revoke(store: &AuthStore, prefix: &str) -> i32 {
    // Look up name first for the confirmation message
    let key_name = store.list_keys().ok().and_then(|keys| {
        keys.into_iter()
            .find(|k| k.prefix.as_str() == prefix)
            .map(|k| k.name)
    });

    match store.revoke_key(prefix) {
        Ok(true) => {
            let name = key_name.unwrap_or_else(|| "unknown".to_string());
            println!("Revoked: {name} ({prefix}). No longer accepts requests.");
            0
        }
        Ok(false) => {
            eprintln!("No key found with prefix: {prefix}");
            1
        }
        Err(e) => {
            eprintln!("error: {e:#}");
            1
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
