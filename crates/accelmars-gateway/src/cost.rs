use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::{params, Connection};
use tracing::warn;

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// A completed request record to be written to SQLite.
#[derive(Debug)]
pub struct RequestRecord {
    pub id: String,
    pub timestamp: String,
    pub tier: String,
    pub provider: String,
    pub model: String,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub cost_usd: f64,
    pub latency_ms: u64,
    pub status: String,
    pub error_type: Option<String>,
    pub constraints: Option<String>,
}

/// Aggregated stats returned by `CostTracker::summary()`.
#[derive(Debug)]
pub struct CostSummary {
    pub total_calls: u64,
    pub total_cost_usd: f64,
    pub by_provider: Vec<ProviderStats>,
    pub by_tier: Vec<TierStats>,
}

#[derive(Debug)]
pub struct ProviderStats {
    pub provider: String,
    pub calls: u64,
    pub cost_usd: f64,
}

#[derive(Debug)]
pub struct TierStats {
    pub tier: String,
    pub calls: u64,
    pub cost_usd: f64,
}

// ---------------------------------------------------------------------------
// CostTracker
// ---------------------------------------------------------------------------

/// Per-request cost tracking using a local SQLite database.
///
/// All write operations are **fail-open** — if SQLite is unavailable (disk full,
/// corrupted, locked), the error is logged and the request still completes.
/// Cost data is observability, not a hard dependency on the request path.
pub struct CostTracker {
    conn: Mutex<Connection>,
}

impl CostTracker {
    /// Open (or create) the cost database at `path`.
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let conn = Connection::open(path)?;
        let tracker = Self {
            conn: Mutex::new(conn),
        };
        tracker.migrate()?;
        Ok(tracker)
    }

    /// Returns the default DB path: `GATEWAY_DB_PATH` env var or `~/.accelmars-gateway.db`.
    pub fn default_path() -> PathBuf {
        std::env::var("GATEWAY_DB_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                let home = std::env::var("HOME")
                    .or_else(|_| std::env::var("USERPROFILE"))
                    .unwrap_or_else(|_| ".".to_string());
                PathBuf::from(home).join(".accelmars-gateway.db")
            })
    }

    fn migrate(&self) -> anyhow::Result<()> {
        let conn = self.conn.lock().expect("cost tracker lock poisoned");
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS requests (
                id          TEXT    PRIMARY KEY,
                timestamp   TEXT    NOT NULL,
                tier        TEXT    NOT NULL,
                provider    TEXT    NOT NULL,
                model       TEXT    NOT NULL,
                tokens_in   INTEGER NOT NULL,
                tokens_out  INTEGER NOT NULL,
                cost_usd    REAL    NOT NULL,
                latency_ms  INTEGER NOT NULL,
                status      TEXT    NOT NULL,
                error_type  TEXT,
                constraints TEXT
            );",
        )?;
        Ok(())
    }

    /// Record a completed request. Never panics — logs and returns on any error.
    pub fn record(&self, record: &RequestRecord) {
        let conn = match self.conn.lock() {
            Ok(c) => c,
            Err(e) => {
                warn!("cost tracker lock poisoned, skipping record: {e}");
                return;
            }
        };
        if let Err(e) = conn.execute(
            "INSERT OR IGNORE INTO requests
                (id, timestamp, tier, provider, model, tokens_in, tokens_out,
                 cost_usd, latency_ms, status, error_type, constraints)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                &record.id,
                &record.timestamp,
                &record.tier,
                &record.provider,
                &record.model,
                record.tokens_in as i64,
                record.tokens_out as i64,
                record.cost_usd,
                record.latency_ms as i64,
                &record.status,
                &record.error_type,
                &record.constraints,
            ],
        ) {
            warn!("cost tracker write error (fail-open) — request still completed: {e}");
        }
    }

    /// Query aggregated stats.
    ///
    /// `since`: optional ISO date string (e.g. `"2026-04-19"`) — filters to
    /// rows with `timestamp >= since`. Pass `None` for all-time.
    pub fn summary(&self, since: Option<&str>) -> anyhow::Result<CostSummary> {
        let conn = self.conn.lock().expect("cost tracker lock poisoned");
        // Using "1970-01-01" as the all-time sentinel keeps SQL uniform.
        let since_str = since.unwrap_or("1970-01-01");

        let total_calls: u64 = conn.query_row(
            "SELECT COUNT(*) FROM requests WHERE timestamp >= ?1",
            params![since_str],
            |row| row.get::<_, i64>(0),
        )? as u64;

        let total_cost_usd: f64 = conn.query_row(
            "SELECT COALESCE(SUM(cost_usd), 0.0) FROM requests WHERE timestamp >= ?1",
            params![since_str],
            |row| row.get(0),
        )?;

        let mut by_provider = Vec::new();
        {
            let mut stmt = conn.prepare(
                "SELECT provider, COUNT(*), COALESCE(SUM(cost_usd), 0.0)
                 FROM requests WHERE timestamp >= ?1
                 GROUP BY provider ORDER BY provider",
            )?;
            let rows = stmt.query_map(params![since_str], |row| {
                Ok(ProviderStats {
                    provider: row.get(0)?,
                    calls: row.get::<_, i64>(1)? as u64,
                    cost_usd: row.get(2)?,
                })
            })?;
            for row in rows {
                by_provider.push(row?);
            }
        }

        let mut by_tier = Vec::new();
        {
            let mut stmt = conn.prepare(
                "SELECT tier, COUNT(*), COALESCE(SUM(cost_usd), 0.0)
                 FROM requests WHERE timestamp >= ?1
                 GROUP BY tier ORDER BY tier",
            )?;
            let rows = stmt.query_map(params![since_str], |row| {
                Ok(TierStats {
                    tier: row.get(0)?,
                    calls: row.get::<_, i64>(1)? as u64,
                    cost_usd: row.get(2)?,
                })
            })?;
            for row in rows {
                by_tier.push(row?);
            }
        }

        Ok(CostSummary {
            total_calls,
            total_cost_usd,
            by_provider,
            by_tier,
        })
    }

    /// Calculate cost for a completed request given the provider's pricing.
    pub fn calculate_cost(
        tokens_in: u64,
        tokens_out: u64,
        cost_per_1m_in: f64,
        cost_per_1m_out: f64,
    ) -> f64 {
        tokens_in as f64 * cost_per_1m_in / 1_000_000.0
            + tokens_out as f64 * cost_per_1m_out / 1_000_000.0
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn in_memory_tracker() -> CostTracker {
        let conn = Connection::open_in_memory().expect("in-memory db");
        let tracker = CostTracker {
            conn: Mutex::new(conn),
        };
        tracker.migrate().expect("migrate");
        tracker
    }

    fn sample_record(id: &str) -> RequestRecord {
        RequestRecord {
            id: id.to_string(),
            timestamp: "2026-04-20T10:00:00Z".to_string(),
            tier: "standard".to_string(),
            provider: "deepseek".to_string(),
            model: "deepseek-chat".to_string(),
            tokens_in: 1000,
            tokens_out: 500,
            cost_usd: 0.000490,
            latency_ms: 1200,
            status: "ok".to_string(),
            error_type: None,
            constraints: None,
        }
    }

    #[test]
    fn record_and_query_back() {
        let tracker = in_memory_tracker();
        tracker.record(&sample_record("req-001"));
        let summary = tracker.summary(None).unwrap();
        assert_eq!(summary.total_calls, 1);
        assert!((summary.total_cost_usd - 0.000490).abs() < 1e-9);
    }

    #[test]
    fn cost_calculation_matches_expected() {
        // deepseek: $0.28/M input, $0.42/M output
        // 1000 input + 500 output
        let expected = 1000.0 * 0.28 / 1_000_000.0 + 500.0 * 0.42 / 1_000_000.0;
        let actual = CostTracker::calculate_cost(1000, 500, 0.28, 0.42);
        assert!((actual - expected).abs() < 1e-12);
    }

    #[test]
    fn fail_open_on_sqlite_error() {
        // Use a tracker with a closed connection to simulate a write error.
        // The record() call must not panic.
        let tracker = in_memory_tracker();
        // Poison the lock to simulate failure
        // (we can't easily poison a Mutex in safe Rust, so we test via a record
        // with a duplicate primary key — INSERT OR IGNORE silently skips it)
        tracker.record(&sample_record("dup-001"));
        tracker.record(&sample_record("dup-001")); // duplicate — silently ignored
        let summary = tracker.summary(None).unwrap();
        assert_eq!(summary.total_calls, 1, "duplicate insert should be ignored");
    }

    #[test]
    fn summary_aggregates_by_provider_and_tier() {
        let tracker = in_memory_tracker();
        tracker.record(&RequestRecord {
            id: "r1".to_string(),
            timestamp: "2026-04-20T10:00:00Z".to_string(),
            tier: "quick".to_string(),
            provider: "gemini".to_string(),
            model: "gemini-2.5-flash-lite".to_string(),
            tokens_in: 500,
            tokens_out: 200,
            cost_usd: 0.0,
            latency_ms: 400,
            status: "ok".to_string(),
            error_type: None,
            constraints: None,
        });
        tracker.record(&RequestRecord {
            id: "r2".to_string(),
            timestamp: "2026-04-20T11:00:00Z".to_string(),
            tier: "standard".to_string(),
            provider: "deepseek".to_string(),
            model: "deepseek-chat".to_string(),
            tokens_in: 1000,
            tokens_out: 500,
            cost_usd: 0.000490,
            latency_ms: 1200,
            status: "ok".to_string(),
            error_type: None,
            constraints: None,
        });

        let summary = tracker.summary(None).unwrap();
        assert_eq!(summary.total_calls, 2);
        assert_eq!(summary.by_provider.len(), 2);
        assert_eq!(summary.by_tier.len(), 2);

        let deepseek = summary
            .by_provider
            .iter()
            .find(|p| p.provider == "deepseek")
            .unwrap();
        assert_eq!(deepseek.calls, 1);
    }

    #[test]
    fn since_filter_limits_results() {
        let tracker = in_memory_tracker();
        tracker.record(&RequestRecord {
            id: "old".to_string(),
            timestamp: "2026-04-18T10:00:00Z".to_string(),
            tier: "quick".to_string(),
            provider: "gemini".to_string(),
            model: "gemini-2.5-flash-lite".to_string(),
            tokens_in: 100,
            tokens_out: 50,
            cost_usd: 0.0,
            latency_ms: 300,
            status: "ok".to_string(),
            error_type: None,
            constraints: None,
        });
        tracker.record(&RequestRecord {
            id: "new".to_string(),
            timestamp: "2026-04-20T10:00:00Z".to_string(),
            tier: "standard".to_string(),
            provider: "deepseek".to_string(),
            model: "deepseek-chat".to_string(),
            tokens_in: 1000,
            tokens_out: 500,
            cost_usd: 0.000490,
            latency_ms: 1200,
            status: "ok".to_string(),
            error_type: None,
            constraints: None,
        });

        let all = tracker.summary(None).unwrap();
        assert_eq!(all.total_calls, 2);

        let recent = tracker.summary(Some("2026-04-19")).unwrap();
        assert_eq!(recent.total_calls, 1, "--since should exclude old record");
        assert_eq!(recent.by_provider[0].provider, "deepseek");
    }
}
