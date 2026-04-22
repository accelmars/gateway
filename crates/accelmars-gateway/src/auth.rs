//! API key authentication — SQLite-backed key store.
//!
//! Pattern: follows `cost.rs` exactly — `Mutex<Connection>`, fail-open writes,
//! `open(path)` constructor, `default_path()` env-overrideable path.
//!
//! Security notes:
//! - Full key returned ONCE on create, never stored.
//! - Only SHA-256 hash stored in DB.
//! - SHA-256 is fast enough for high-entropy random keys (not passwords).
//!   Same approach as OpenAI, Stripe, Anthropic.
//! - Fail-open on DB error: broken auth DB logs a warning but does not take
//!   down the server. Consistent with cost tracker philosophy.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// API key prefix — first 12 characters of a full key.
///
/// Used for display and identification. Cannot be used to authenticate
/// (only the SHA-256 hash of the full key is stored).
///
/// Validation: must start with `gw_`, must be exactly 12 characters.
#[derive(Debug, Clone)]
pub struct KeyPrefix(String);

impl KeyPrefix {
    pub fn new(s: impl Into<String>) -> Result<Self, String> {
        let s = s.into();
        if !s.starts_with("gw_") || s.len() != 12 {
            return Err(format!(
                "invalid key prefix '{}': must start with 'gw_' and be exactly 12 chars",
                s
            ));
        }
        Ok(Self(s))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for KeyPrefix {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A stored API key record (no plaintext key — only hash + prefix).
#[derive(Debug, Clone)]
pub struct ApiKeyRecord {
    pub id: String,
    pub key_hash: String,
    pub prefix: KeyPrefix,
    pub name: String,
    pub created_at: String,
    pub last_used: Option<String>,
    pub active: bool,
}

// ---------------------------------------------------------------------------
// AuthStore
// ---------------------------------------------------------------------------

/// SQLite-backed API key store.
///
/// All write operations are **fail-open** — errors are logged but do not
/// block the request path. Consistent with `CostTracker`.
pub struct AuthStore {
    conn: Mutex<Connection>,
}

impl AuthStore {
    /// Open (or create) the auth database at `path`.
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let conn = Connection::open(path)?;
        let store = Self {
            conn: Mutex::new(conn),
        };
        store.migrate()?;
        Ok(store)
    }

    /// Open an in-memory database — for tests only.
    pub fn in_memory() -> anyhow::Result<Self> {
        let conn = Connection::open_in_memory()?;
        let store = Self {
            conn: Mutex::new(conn),
        };
        store.migrate()?;
        Ok(store)
    }

    /// Default DB path: `GATEWAY_AUTH_DB_PATH` env var or `~/.accelmars-gateway-auth.db`.
    pub fn default_path() -> PathBuf {
        std::env::var("GATEWAY_AUTH_DB_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                let home = std::env::var("HOME")
                    .or_else(|_| std::env::var("USERPROFILE"))
                    .unwrap_or_else(|_| ".".to_string());
                PathBuf::from(home).join(".accelmars-gateway-auth.db")
            })
    }

    fn migrate(&self) -> anyhow::Result<()> {
        let conn = self.conn.lock().expect("auth store lock poisoned");
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS api_keys (
                id          TEXT    PRIMARY KEY,
                key_hash    TEXT    NOT NULL UNIQUE,
                prefix      TEXT    NOT NULL,
                name        TEXT    NOT NULL,
                created_at  TEXT    NOT NULL,
                last_used   TEXT,
                active      INTEGER NOT NULL DEFAULT 1
            );",
        )?;
        Ok(())
    }

    /// Create a new API key with the given human-readable name.
    ///
    /// Returns `(full_key, record)`. The `full_key` is shown ONCE — never stored.
    /// The `record.key_hash` is the SHA-256 of the full key.
    pub fn create_key(&self, name: &str) -> anyhow::Result<(String, ApiKeyRecord)> {
        let full_key = generate_key();
        let key_hash = hash_key(&full_key);
        // First 12 chars of key form the display prefix ("gw_live_XXXX")
        let prefix_str = &full_key[..12];
        let prefix = KeyPrefix::new(prefix_str).expect("generated key always has valid prefix");
        let id = uuid::Uuid::new_v4().to_string();
        let created_at = iso_now();

        let conn = self.conn.lock().expect("auth store lock poisoned");
        conn.execute(
            "INSERT INTO api_keys (id, key_hash, prefix, name, created_at, active)
             VALUES (?1, ?2, ?3, ?4, ?5, 1)",
            params![&id, &key_hash, prefix.as_str(), name, &created_at],
        )?;

        let record = ApiKeyRecord {
            id,
            key_hash,
            prefix,
            name: name.to_string(),
            created_at,
            last_used: None,
            active: true,
        };

        Ok((full_key, record))
    }

    /// Validate a key. Returns `Some(record)` if valid and active, `None` if invalid or revoked.
    ///
    /// Updates `last_used` on success.
    pub fn validate_key(&self, key: &str) -> anyhow::Result<Option<ApiKeyRecord>> {
        let key_hash = hash_key(key);
        let conn = self.conn.lock().expect("auth store lock poisoned");

        let result = conn.query_row(
            "SELECT id, key_hash, prefix, name, created_at, last_used, active
             FROM api_keys WHERE key_hash = ?1",
            params![&key_hash],
            |row| {
                let active: i64 = row.get(6)?;
                Ok(ApiKeyRecord {
                    id: row.get(0)?,
                    key_hash: row.get(1)?,
                    prefix: KeyPrefix(row.get::<_, String>(2)?),
                    name: row.get(3)?,
                    created_at: row.get(4)?,
                    last_used: row.get(5)?,
                    active: active != 0,
                })
            },
        );

        match result {
            Ok(record) if record.active => {
                let now = iso_now();
                let _ = conn.execute(
                    "UPDATE api_keys SET last_used = ?1 WHERE key_hash = ?2",
                    params![&now, &key_hash],
                );
                Ok(Some(record))
            }
            Ok(_) => Ok(None), // revoked
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// List all keys (active and revoked), ordered by creation time descending.
    pub fn list_keys(&self) -> anyhow::Result<Vec<ApiKeyRecord>> {
        let conn = self.conn.lock().expect("auth store lock poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, key_hash, prefix, name, created_at, last_used, active
             FROM api_keys ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            let active: i64 = row.get(6)?;
            Ok(ApiKeyRecord {
                id: row.get(0)?,
                key_hash: row.get(1)?,
                prefix: KeyPrefix(row.get::<_, String>(2)?),
                name: row.get(3)?,
                created_at: row.get(4)?,
                last_used: row.get(5)?,
                active: active != 0,
            })
        })?;

        let mut records = Vec::new();
        for row in rows {
            records.push(row?);
        }
        Ok(records)
    }

    /// Revoke a key by prefix. Returns `true` if found and revoked, `false` if not found.
    pub fn revoke_key(&self, prefix: &str) -> anyhow::Result<bool> {
        let conn = self.conn.lock().expect("auth store lock poisoned");
        let affected = conn.execute(
            "UPDATE api_keys SET active = 0 WHERE prefix = ?1",
            params![prefix],
        )?;
        Ok(affected > 0)
    }
}

// ---------------------------------------------------------------------------
// Key generation and hashing
// ---------------------------------------------------------------------------

fn generate_key() -> String {
    use rand::Rng;
    let random: String = rand::thread_rng()
        .sample_iter(&rand::distributions::Alphanumeric)
        .take(32)
        .map(char::from)
        .collect();
    format!("gw_live_{random}")
}

fn hash_key(key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(key.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Minimal ISO-8601 UTC timestamp — same approach as `pid.rs`, no chrono dependency.
fn iso_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let sec = (secs % 60) as u32;
    let secs = secs / 60;
    let min = (secs % 60) as u32;
    let secs = secs / 60;
    let hour = (secs % 24) as u32;
    let mut days = secs / 24;
    let mut year = 1970u32;
    loop {
        let diy: u64 = if is_leap(year) { 366 } else { 365 };
        if days < diy {
            break;
        }
        days -= diy;
        year += 1;
    }
    let leap = is_leap(year);
    let month_days: [u64; 12] = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut month = 1u32;
    for &md in &month_days {
        if days < md {
            break;
        }
        days -= md;
        month += 1;
    }
    let day = days as u32 + 1;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{min:02}:{sec:02}Z")
}

fn is_leap(year: u32) -> bool {
    year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400))
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn test_store() -> AuthStore {
        AuthStore::in_memory().expect("in-memory auth store")
    }

    // Test 1: Create key returns gw_live_ prefix + 32 chars (40 total)
    #[test]
    fn create_key_returns_gw_live_prefix() {
        let store = test_store();
        let (key, record) = store.create_key("test").unwrap();
        assert!(
            key.starts_with("gw_live_"),
            "key should start with gw_live_, got: {key}"
        );
        assert_eq!(
            key.len(),
            40,
            "key should be 40 chars (gw_live_ + 32), got: {}",
            key.len()
        );
        assert!(record.active);
        assert_eq!(record.name, "test");
    }

    // Test 2: Hash stored, not plaintext
    #[test]
    fn create_key_stores_hash_not_plaintext() {
        let store = test_store();
        let (key, record) = store.create_key("hash-test").unwrap();
        assert_ne!(
            record.key_hash, key,
            "stored hash must not equal plaintext key"
        );
        assert_eq!(record.key_hash.len(), 64, "SHA-256 hex digest is 64 chars");
        assert!(
            !record.key_hash.contains("gw_"),
            "plaintext prefix must not appear in stored hash"
        );
    }

    // Test 3: Validate correct key returns Some(record)
    #[test]
    fn validate_correct_key_returns_some() {
        let store = test_store();
        let (key, created) = store.create_key("validate-test").unwrap();
        let result = store.validate_key(&key).unwrap();
        assert!(result.is_some(), "valid key should return Some");
        let record = result.unwrap();
        assert_eq!(record.id, created.id);
        assert_eq!(record.name, "validate-test");
    }

    // Test 4: Validate wrong key returns None
    #[test]
    fn validate_wrong_key_returns_none() {
        let store = test_store();
        let _ = store.create_key("real").unwrap();
        let result = store
            .validate_key("gw_live_wrongkeyAAAAAAAAAAAAAAAAAAAAAAAA")
            .unwrap();
        assert!(result.is_none(), "invalid key should return None");
    }

    // Test 5: Validate revoked key returns None
    #[test]
    fn validate_revoked_key_returns_none() {
        let store = test_store();
        let (key, record) = store.create_key("to-revoke").unwrap();
        store.revoke_key(record.prefix.as_str()).unwrap();
        let result = store.validate_key(&key).unwrap();
        assert!(result.is_none(), "revoked key should return None");
    }

    // Test 6: Revoke sets active = 0
    #[test]
    fn revoke_key_sets_inactive() {
        let store = test_store();
        let (_, record) = store.create_key("revoke-me").unwrap();
        let found = store.revoke_key(record.prefix.as_str()).unwrap();
        assert!(found, "revoke should return true when key is found");
        let keys = store.list_keys().unwrap();
        let k = keys.iter().find(|k| k.id == record.id).unwrap();
        assert!(!k.active, "key should be inactive after revoke");
    }

    // Test 7: List keys returns all (active and revoked)
    #[test]
    fn list_keys_returns_all() {
        let store = test_store();
        let (_, r1) = store.create_key("key-1").unwrap();
        let (_, r2) = store.create_key("key-2").unwrap();
        store.revoke_key(r2.prefix.as_str()).unwrap();
        let keys = store.list_keys().unwrap();
        assert_eq!(
            keys.len(),
            2,
            "list should include both active and revoked keys"
        );
        assert!(
            keys.iter().any(|k| k.id == r1.id),
            "active key should be in list"
        );
        assert!(
            keys.iter().any(|k| k.id == r2.id),
            "revoked key should be in list"
        );
    }

    // Test 8: last_used updated on validate
    #[test]
    fn validate_updates_last_used() {
        let store = test_store();
        let (key, record) = store.create_key("usage-track").unwrap();
        assert!(
            record.last_used.is_none(),
            "last_used should be None before first use"
        );
        store.validate_key(&key).unwrap();
        let keys = store.list_keys().unwrap();
        let k = keys.iter().find(|k| k.id == record.id).unwrap();
        assert!(
            k.last_used.is_some(),
            "last_used should be set after validation"
        );
    }

    // Test 9: Duplicate name allowed (keys unique by hash, not name)
    #[test]
    fn duplicate_name_allowed() {
        let store = test_store();
        let r1 = store.create_key("same-name");
        let r2 = store.create_key("same-name");
        assert!(r1.is_ok(), "first key with name should succeed");
        assert!(r2.is_ok(), "second key with same name should also succeed");
        assert_eq!(store.list_keys().unwrap().len(), 2);
    }

    // Test 10: KeyPrefix validation rejects strings not starting with gw_
    #[test]
    fn key_prefix_validation() {
        // Invalid cases
        assert!(KeyPrefix::new("sk_live_test").is_err(), "wrong prefix");
        assert!(KeyPrefix::new("gw_short").is_err(), "too short");
        assert!(
            KeyPrefix::new("gw_toolongstri").is_err(),
            "too long (14 chars)"
        );
        assert!(KeyPrefix::new("").is_err(), "empty string");

        // Valid case: exactly 12 chars starting with gw_
        let p = KeyPrefix::new("gw_live_a8f2");
        assert!(
            p.is_ok(),
            "valid 12-char prefix starting with gw_ should pass"
        );
        assert_eq!(p.unwrap().as_str(), "gw_live_a8f2");
    }
}
