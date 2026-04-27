use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use nutype::nutype;
use rand::rngs::SmallRng;
use rand::{Rng, SeedableRng};
use serde::{Deserialize, Serialize};

use accelmars_gateway_core::{GatewayRequest, ModelTier, ProviderAdapter};

// ---------------------------------------------------------------------------
// Domain newtypes
// ---------------------------------------------------------------------------

/// Traffic weight for canary routing. Valid range: 0–100 (inclusive).
/// 0 = canary disabled. 100 = all traffic to canary.
#[nutype(
    validate(less_or_equal = 100),
    derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)
)]
pub struct WeightPercent(u8);

/// Error rate threshold for automatic canary rollback. Valid range: 0.0–1.0.
#[nutype(
    validate(finite, less_or_equal = 1.0, greater_or_equal = 0.0),
    derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)
)]
pub struct ErrorThreshold(f64);

// ---------------------------------------------------------------------------
// CandidateConfig
// Lives here (not config.rs) to avoid circular import: config.rs imports canary
// types, while canary.rs would need config.rs types — circular.
// ---------------------------------------------------------------------------

/// Configuration for a canary or shadow candidate provider.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CandidateConfig {
    pub provider: String,
    pub weight_percent: WeightPercent,
    #[serde(default)]
    pub shadow_mode: bool,
    /// Rolling window size for rollback health tracking. Default: 100.
    #[serde(default)]
    pub rollback_window: Option<usize>,
    /// Error rate threshold that triggers rollback. Default: 0.20. Validated ≥ 0.0 at use time.
    #[serde(default)]
    pub rollback_threshold: Option<f64>,
    /// Shadow queue capacity. Default: 1000.
    #[serde(default)]
    pub shadow_queue_capacity: Option<usize>,
}

// ---------------------------------------------------------------------------
// Canary phase state machine
// ---------------------------------------------------------------------------

/// Reason a canary was rolled back.
#[derive(Debug, Clone)]
pub enum RollbackReason {
    ErrorRateExceeded {
        window_errors: usize,
        window_size: usize,
    },
    ManualOverride,
}

/// Canary phase — valid transitions:
///   Stable → Monitoring     (canary config loaded at startup or re-enabled)
///   Monitoring → RollingBack (error threshold exceeded in rolling window)
///   Monitoring → Stable      (weight set to 0 / manual disable)
///   RollingBack → Stable     (rollback complete — manual re-enable)
///
/// INVALID: RollingBack → Monitoring (must pass through Stable)
#[derive(Debug, Clone)]
pub enum CanaryPhase {
    Stable,
    Monitoring,
    RollingBack { reason: RollbackReason },
}

// ---------------------------------------------------------------------------
// CanaryHealthTracker
// ---------------------------------------------------------------------------

/// Rolling-window error rate tracker for automatic canary rollback.
pub struct CanaryHealthTracker {
    window: VecDeque<bool>,
    window_size: usize,
    error_threshold_val: f64,
}

impl CanaryHealthTracker {
    pub fn new(window_size: usize, error_threshold: f64) -> Self {
        Self {
            window: VecDeque::new(),
            window_size,
            error_threshold_val: error_threshold,
        }
    }

    pub fn record(&mut self, success: bool) {
        if self.window.len() == self.window_size {
            self.window.pop_front();
        }
        self.window.push_back(success);
    }

    pub fn error_rate(&self) -> f64 {
        if self.window.is_empty() {
            return 0.0;
        }
        let errors = self.window.iter().filter(|&&b| !b).count();
        errors as f64 / self.window.len() as f64
    }

    /// Returns true when the window is full and error rate strictly exceeds the threshold.
    pub fn should_rollback(&self) -> bool {
        self.window.len() == self.window_size && self.error_rate() > self.error_threshold_val
    }

    pub fn window_error_count(&self) -> usize {
        self.window.iter().filter(|&&b| !b).count()
    }

    pub fn window_len(&self) -> usize {
        self.window.len()
    }
}

// ---------------------------------------------------------------------------
// CanaryState
// ---------------------------------------------------------------------------

/// All runtime state for a single tier's canary deployment.
/// Lives inside Arc<> in Router — shared across all request handlers.
pub struct CanaryState {
    pub phase: RwLock<CanaryPhase>,
    pub candidate: CandidateConfig,
    pub tracker: Mutex<CanaryHealthTracker>,
    pub active: AtomicBool,
}

impl CanaryState {
    pub fn new(candidate: CandidateConfig) -> Self {
        let window_size = candidate.rollback_window.unwrap_or(100);
        let threshold = candidate.rollback_threshold.unwrap_or(0.20);

        if candidate.rollback_window.is_none() || candidate.rollback_threshold.is_none() {
            tracing::info!(
                window = window_size,
                threshold = threshold,
                "canary defaults applied"
            );
        }

        let active = candidate.weight_percent.into_inner() > 0;
        let initial_phase = if active {
            CanaryPhase::Monitoring
        } else {
            CanaryPhase::Stable
        };

        Self {
            phase: RwLock::new(initial_phase),
            candidate,
            tracker: Mutex::new(CanaryHealthTracker::new(window_size, threshold)),
            active: AtomicBool::new(active),
        }
    }

    /// Transition to the next phase. Enforces valid transition table.
    pub fn transition_to(&self, next: CanaryPhase) {
        let mut phase = self.phase.write().expect("canary phase lock poisoned");

        // RollingBack → Monitoring is invalid: must pass through Stable to re-enable
        if matches!(*phase, CanaryPhase::RollingBack { .. })
            && matches!(next, CanaryPhase::Monitoring)
        {
            debug_assert!(false, "invalid canary transition: RollingBack → Monitoring");
            return;
        }

        tracing::info!(from = ?*phase, to = ?next, "canary phase transition");
        *phase = next;
    }

    /// Record a canary request result. Only active in Monitoring phase.
    /// Triggers rollback if error rate exceeds threshold with a full window.
    pub fn record_result(&self, success: bool) {
        let is_monitoring = {
            let p = self.phase.read().expect("canary phase lock poisoned");
            matches!(*p, CanaryPhase::Monitoring)
        };
        if !is_monitoring {
            return;
        }

        let (should_roll, window_errors, window_size) = {
            let mut tracker = self.tracker.lock().expect("canary tracker lock poisoned");
            tracker.record(success);
            (
                tracker.should_rollback(),
                tracker.window_error_count(),
                tracker.window_len(),
            )
        };

        if should_roll {
            self.transition_to(CanaryPhase::RollingBack {
                reason: RollbackReason::ErrorRateExceeded {
                    window_errors,
                    window_size,
                },
            });
            self.active.store(false, Ordering::Release);
        }
    }
}

// ---------------------------------------------------------------------------
// ShadowTask and ShadowQueueSender
// ---------------------------------------------------------------------------

pub struct ShadowTask {
    pub adapter: Arc<dyn ProviderAdapter>,
    pub request: GatewayRequest,
    pub tier: ModelTier,
    pub provider_name: String,
}

pub type ShadowQueueSender = tokio::sync::mpsc::Sender<ShadowTask>;

pub const SHADOW_QUEUE_DEFAULT_CAPACITY: usize = 1000;

// ---------------------------------------------------------------------------
// CanaryRng trait
// ---------------------------------------------------------------------------

/// Abstraction for the weighted-random selection used in canary routing.
/// Mockable for deterministic Crucible eval runs.
pub trait CanaryRng: Send + Sync {
    fn should_take_canary(&self, weight_percent: WeightPercent) -> bool;
}

/// Default: non-deterministic RNG initialized from OS entropy at server startup.
pub struct DefaultCanaryRng {
    rng: Mutex<SmallRng>,
}

impl DefaultCanaryRng {
    pub fn new() -> Self {
        Self {
            rng: Mutex::new(SmallRng::from_entropy()),
        }
    }
}

impl Default for DefaultCanaryRng {
    fn default() -> Self {
        Self::new()
    }
}

impl CanaryRng for DefaultCanaryRng {
    fn should_take_canary(&self, weight_percent: WeightPercent) -> bool {
        let mut rng = self.rng.lock().expect("canary rng lock poisoned");
        rng.gen_range(0u8..100) < weight_percent.into_inner()
    }
}

/// Seeded: deterministic RNG for Crucible reproducibility.
pub struct SeededCanaryRng {
    pub seed: u64,
}

impl CanaryRng for SeededCanaryRng {
    fn should_take_canary(&self, weight_percent: WeightPercent) -> bool {
        let mut rng = SmallRng::seed_from_u64(self.seed);
        rng.gen_range(0u8..100) < weight_percent.into_inner()
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weight_percent_rejects_above_100() {
        assert!(WeightPercent::new(101).is_err());
        assert!(WeightPercent::new(100).is_ok());
        assert!(WeightPercent::new(0).is_ok());
    }

    #[test]
    fn error_threshold_rejects_above_1() {
        assert!(ErrorThreshold::new(1.1).is_err());
        assert!(ErrorThreshold::new(1.0).is_ok());
        assert!(ErrorThreshold::new(0.0).is_ok());
    }

    #[test]
    fn rollback_triggers_on_error_threshold() {
        let mut tracker = CanaryHealthTracker::new(5, 0.20);

        // Record 4 successes then 1 failure — window full, error_rate = 1/5 = 20% (at threshold)
        for _ in 0..4 {
            tracker.record(true);
        }
        tracker.record(false);
        // At threshold (0.20), not strictly over it — should_rollback() uses strict >
        assert!(
            !tracker.should_rollback(),
            "should not rollback at exact threshold (strict > required)"
        );

        // One more failure pops oldest success — window becomes [true, true, true, false, false]
        // error_rate = 2/5 = 0.40 > 0.20 → rollback triggered
        tracker.record(false);
        assert!(
            tracker.should_rollback(),
            "should rollback when error rate exceeds threshold"
        );
    }

    #[test]
    fn seeded_rng_deterministic() {
        let rng = SeededCanaryRng { seed: 42 };
        let full_weight = WeightPercent::new(100).unwrap();
        let zero_weight = WeightPercent::new(0).unwrap();

        assert!(
            rng.should_take_canary(full_weight),
            "weight=100 must always take canary"
        );
        assert!(
            !rng.should_take_canary(zero_weight),
            "weight=0 must never take canary"
        );
    }
}
