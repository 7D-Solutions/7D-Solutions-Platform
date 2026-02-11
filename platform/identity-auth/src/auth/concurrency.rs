use std::sync::Arc;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::time::{timeout, Duration};

#[derive(Clone)]
pub struct HashConcurrencyLimiter {
    sem: Arc<Semaphore>,
    acquire_timeout: Duration,
}

#[derive(Debug)]
pub enum AcquireError {
    Timeout,
}

impl HashConcurrencyLimiter {
    pub fn new(max_concurrent: usize, acquire_timeout_ms: u64) -> Self {
        let max = max_concurrent.max(1);
        Self {
            sem: Arc::new(Semaphore::new(max)),
            acquire_timeout: Duration::from_millis(acquire_timeout_ms.max(1)),
        }
    }

    pub async fn acquire(&self) -> Result<OwnedSemaphorePermit, AcquireError> {
        match timeout(self.acquire_timeout, self.sem.clone().acquire_owned()).await {
            Ok(Ok(permit)) => Ok(permit),
            _ => Err(AcquireError::Timeout),
        }
    }
}
