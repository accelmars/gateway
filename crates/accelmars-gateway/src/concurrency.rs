use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::time::timeout;

const DEFAULT_QUEUE_TIMEOUT_SECS: u64 = 30;

/// Error returned when a permit could not be acquired in time.
#[derive(Debug, thiserror::Error)]
pub enum ConcurrencyError {
    #[error("no permit available after {0}s — too many concurrent requests")]
    QueueTimeout(u64),
}

/// Global concurrency semaphore — limits parallel AI calls to configured max.
///
/// Requests beyond the limit queue until a slot opens or the timeout expires.
/// On timeout the caller should return HTTP 504.
pub struct ConcurrencyLimiter {
    semaphore: Arc<Semaphore>,
    max_permits: usize,
    queue_timeout: Duration,
}

impl ConcurrencyLimiter {
    pub fn new(max_concurrent: usize) -> Self {
        Self::with_timeout(
            max_concurrent,
            Duration::from_secs(DEFAULT_QUEUE_TIMEOUT_SECS),
        )
    }

    pub fn with_timeout(max_concurrent: usize, queue_timeout: Duration) -> Self {
        Self {
            semaphore: Arc::new(Semaphore::new(max_concurrent)),
            max_permits: max_concurrent,
            queue_timeout,
        }
    }

    /// Acquire a permit. Blocks until a slot opens, or returns `QueueTimeout`.
    ///
    /// The returned permit must be held for the duration of the AI call.
    /// Dropping it releases the slot for the next queued request.
    pub async fn acquire(&self) -> Result<OwnedSemaphorePermit, ConcurrencyError> {
        timeout(
            self.queue_timeout,
            Arc::clone(&self.semaphore).acquire_owned(),
        )
        .await
        .map_err(|_| ConcurrencyError::QueueTimeout(self.queue_timeout.as_secs()))?
        .map_err(|_| ConcurrencyError::QueueTimeout(self.queue_timeout.as_secs()))
    }

    /// Number of permits currently in use (active AI calls).
    pub fn active(&self) -> usize {
        self.max_permits
            .saturating_sub(self.semaphore.available_permits())
    }

    /// Number of permits available (slots open for new requests).
    pub fn available(&self) -> usize {
        self.semaphore.available_permits()
    }

    /// Configured max concurrent AI calls.
    pub fn max(&self) -> usize {
        self.max_permits
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[tokio::test]
    async fn permits_up_to_max_acquired_immediately() {
        let limiter = ConcurrencyLimiter::new(3);
        let _p1 = limiter.acquire().await.unwrap();
        let _p2 = limiter.acquire().await.unwrap();
        let _p3 = limiter.acquire().await.unwrap();
        assert_eq!(limiter.active(), 3);
        assert_eq!(limiter.available(), 0);
    }

    #[tokio::test]
    async fn fourth_request_queues_and_completes_when_slot_opens() {
        let limiter = Arc::new(ConcurrencyLimiter::new(3));

        let p1 = limiter.acquire().await.unwrap();
        let _p2 = limiter.acquire().await.unwrap();
        let _p3 = limiter.acquire().await.unwrap();

        // Spawn the 4th acquire — it will block until p1 is dropped
        let limiter2 = Arc::clone(&limiter);
        let handle = tokio::spawn(async move { limiter2.acquire().await });

        // Let the task start
        tokio::task::yield_now().await;

        // Release slot 1
        drop(p1);

        // 4th request should now succeed
        let result = handle.await.unwrap();
        assert!(
            result.is_ok(),
            "4th request should succeed when a slot opens"
        );
    }

    #[tokio::test]
    async fn queue_timeout_returns_error() {
        // Create a limiter with 1 permit and a very short timeout
        let limiter = Arc::new(ConcurrencyLimiter::with_timeout(
            1,
            Duration::from_millis(50),
        ));

        let _p1 = limiter.acquire().await.unwrap(); // holds the only permit

        // Second acquire should time out
        let err = limiter.acquire().await.unwrap_err();
        assert!(
            matches!(err, ConcurrencyError::QueueTimeout(_)),
            "expected QueueTimeout, got {err:?}"
        );
    }

    #[tokio::test]
    async fn active_and_available_track_correctly() {
        let limiter = ConcurrencyLimiter::new(5);
        assert_eq!(limiter.active(), 0);
        assert_eq!(limiter.available(), 5);
        assert_eq!(limiter.max(), 5);

        let _p = limiter.acquire().await.unwrap();
        assert_eq!(limiter.active(), 1);
        assert_eq!(limiter.available(), 4);
    }
}
