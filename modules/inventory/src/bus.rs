use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc, Mutex,
};

/// Tracks the health of the configured event bus.
#[derive(Debug)]
pub struct BusHealth {
    connected: AtomicBool,
    latency_ms: AtomicU64,
    last_error: Mutex<Option<String>>,
}

impl BusHealth {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            connected: AtomicBool::new(false),
            latency_ms: AtomicU64::new(0),
            last_error: Mutex::new(None),
        })
    }

    pub fn mark_connected(&self, latency_ms: u64) {
        self.connected.store(true, Ordering::Relaxed);
        self.latency_ms.store(latency_ms, Ordering::Relaxed);
        if let Ok(mut err) = self.last_error.lock() {
            *err = None;
        }
    }

    pub fn mark_disconnected(&self, error: Option<String>, latency_ms: u64) {
        self.connected.store(false, Ordering::Relaxed);
        self.latency_ms.store(latency_ms, Ordering::Relaxed);
        if let Ok(mut err) = self.last_error.lock() {
            *err = error;
        }
    }

    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }

    pub fn latency_ms(&self) -> u64 {
        self.latency_ms.load(Ordering::Relaxed)
    }

    pub fn last_error(&self) -> Option<String> {
        self.last_error.lock().ok().and_then(|err| err.clone())
    }
}
