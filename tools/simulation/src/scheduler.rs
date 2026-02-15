//! Concurrent Scheduler for Simulation (bd-3c2)
//!
//! **Purpose:** Parallel execution with barrier synchronization
//!
//! **ChatGPT Requirements:**
//! - 8-32 scheduler workers
//! - Barrier start (no artificial ordering)
//! - No sleeps
//! - Rely on DB UNIQUE, FOR UPDATE, idempotency keys, pure guards

use std::sync::Arc;
use tokio::sync::Barrier;
use tokio::task::JoinSet;
use tracing::{debug, warn};

/// Concurrent scheduler with barrier synchronization
pub struct ConcurrentScheduler {
    /// Number of workers
    worker_count: usize,
}

impl ConcurrentScheduler {
    /// Create new scheduler with worker count
    ///
    /// **ChatGPT Requirement:** 8-32 workers
    pub fn new(worker_count: usize) -> Self {
        assert!(worker_count >= 8 && worker_count <= 32,
            "Worker count must be 8-32, got {}", worker_count);
        Self { worker_count }
    }

    /// Execute tasks concurrently with barrier synchronization
    ///
    /// **Pattern:**
    /// 1. All workers reach barrier
    /// 2. All workers start simultaneously (no artificial ordering)
    /// 3. Each worker executes task independently
    /// 4. Wait for all workers to complete
    ///
    /// **Parameters:**
    /// - `tasks`: Vec of async tasks to execute
    ///
    /// **Returns:** Vec of results (in arbitrary order)
    pub async fn execute_concurrent<F, Fut, T>(
        &self,
        tasks: Vec<F>,
    ) -> Vec<Result<T, String>>
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<T, String>> + Send,
        T: Send + 'static,
    {
        let task_count = tasks.len();
        let barrier = Arc::new(Barrier::new(task_count));
        let mut join_set = JoinSet::new();

        debug!(
            worker_count = self.worker_count,
            task_count = task_count,
            "Starting concurrent execution with barrier"
        );

        for (i, task) in tasks.into_iter().enumerate() {
            let barrier_clone = Arc::clone(&barrier);

            join_set.spawn(async move {
                // Wait at barrier for all workers
                barrier_clone.wait().await;

                debug!(worker_id = i, "Worker started after barrier");

                // Execute task (no sleeps, no artificial ordering)
                task().await
            });
        }

        // Collect results (order not guaranteed)
        let mut results = Vec::with_capacity(task_count);
        while let Some(result) = join_set.join_next().await {
            match result {
                Ok(task_result) => results.push(task_result),
                Err(e) => {
                    warn!(error = %e, "Worker task panicked");
                    results.push(Err(format!("Worker panicked: {}", e)));
                }
            }
        }

        debug!(
            result_count = results.len(),
            "Concurrent execution completed"
        );

        results
    }

    /// Execute single task across multiple workers (stress test)
    ///
    /// **Purpose:** Test concurrency safety (e.g., UNIQUE constraints, FOR UPDATE locks)
    ///
    /// **Pattern:** All workers execute the same operation simultaneously
    /// - Only one should succeed (DB constraints enforce exactly-once)
    /// - Others should fail gracefully (UNIQUE violation, etc.)
    pub async fn execute_stress<Fut, T>(
        &self,
        task_factory: impl Fn() -> Fut + Send + Sync + 'static,
    ) -> Vec<Result<T, String>>
    where
        Fut: std::future::Future<Output = Result<T, String>> + Send + 'static,
        T: Send + 'static,
    {
        let task_factory = Arc::new(task_factory);
        let tasks: Vec<_> = (0..self.worker_count)
            .map(|_| {
                let factory = Arc::clone(&task_factory);
                move || (factory)()
            })
            .collect();

        self.execute_concurrent(tasks).await
    }

    /// Get worker count
    pub fn worker_count(&self) -> usize {
        self.worker_count
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    #[tokio::test]
    async fn test_concurrent_execution() {
        let scheduler = ConcurrentScheduler::new(16);

        let counter = Arc::new(AtomicU32::new(0));
        let tasks: Vec<_> = (0..16)
            .map(|_| {
                let counter_clone = Arc::clone(&counter);
                move || async move {
                    counter_clone.fetch_add(1, Ordering::Relaxed);
                    Ok::<_, String>(())
                }
            })
            .collect();

        let results = scheduler.execute_concurrent(tasks).await;

        assert_eq!(results.len(), 16);
        assert_eq!(counter.load(Ordering::Relaxed), 16);
        assert!(results.iter().all(|r| r.is_ok()));
    }

    #[tokio::test]
    async fn test_barrier_synchronization() {
        let scheduler = ConcurrentScheduler::new(8);

        let start_times = Arc::new(tokio::sync::Mutex::new(Vec::new()));
        let tasks: Vec<_> = (0..8)
            .map(|i| {
                let times = Arc::clone(&start_times);
                move || async move {
                    let now = std::time::Instant::now();
                    times.lock().await.push((i, now));
                    Ok::<_, String>(())
                }
            })
            .collect();

        scheduler.execute_concurrent(tasks).await;

        let times = start_times.lock().await;
        assert_eq!(times.len(), 8);

        // All tasks should start within a very short window (barrier synchronization)
        let first_time = times[0].1;
        let max_diff = times.iter()
            .map(|(_, t)| t.duration_since(first_time))
            .max()
            .unwrap();

        // Should be < 100ms (barrier ensures simultaneous start)
        assert!(max_diff.as_millis() < 100,
            "Tasks should start simultaneously, max diff: {:?}", max_diff);
    }

    #[test]
    #[should_panic(expected = "Worker count must be 8-32")]
    fn test_invalid_worker_count_low() {
        ConcurrentScheduler::new(4);
    }

    #[test]
    #[should_panic(expected = "Worker count must be 8-32")]
    fn test_invalid_worker_count_high() {
        ConcurrentScheduler::new(64);
    }
}
