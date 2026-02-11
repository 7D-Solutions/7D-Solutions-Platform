use dashmap::DashMap;
use governor::{
    clock::{Clock, DefaultClock},
    state::{InMemoryState, NotKeyed},
    Quota, RateLimiter,
};
use std::{num::NonZeroU32, sync::Arc, time::Duration};

#[derive(Clone)]
pub struct KeyedLimiters {
    // key -> limiter
    email_login: Arc<DashMap<String, Arc<RateLimiter<NotKeyed, InMemoryState, DefaultClock>>>>,
    email_register: Arc<DashMap<String, Arc<RateLimiter<NotKeyed, InMemoryState, DefaultClock>>>>,
    refresh: Arc<DashMap<String, Arc<RateLimiter<NotKeyed, InMemoryState, DefaultClock>>>>,
}

impl KeyedLimiters {
    pub fn new() -> Self {
        Self {
            email_login: Arc::new(DashMap::new()),
            email_register: Arc::new(DashMap::new()),
            refresh: Arc::new(DashMap::new()),
        }
    }

    fn limiter_for(map: &DashMap<String, Arc<RateLimiter<NotKeyed, InMemoryState, DefaultClock>>>, key: &str, per_min: u32)
        -> Arc<RateLimiter<NotKeyed, InMemoryState, DefaultClock>>
    {
        if let Some(v) = map.get(key) {
            return v.clone();
        }

        let quota = Quota::per_minute(NonZeroU32::new(per_min.max(1)).unwrap())
            .allow_burst(NonZeroU32::new(per_min.max(1)).unwrap());

        let limiter = Arc::new(RateLimiter::direct(quota));
        map.insert(key.to_string(), limiter.clone());
        limiter
    }

    pub fn check_login_email(&self, tenant_id: &str, email: &str, per_min: u32) -> Result<(), Duration> {
        let key = format!("{}:{}", tenant_id, email);
        let lim = Self::limiter_for(&self.email_login, &key, per_min);
        lim.check().map_err(|n| n.wait_time_from(DefaultClock::default().now()))
    }

    pub fn check_register_email(&self, tenant_id: &str, email: &str, per_min: u32) -> Result<(), Duration> {
        let key = format!("{}:{}", tenant_id, email);
        let lim = Self::limiter_for(&self.email_register, &key, per_min);
        lim.check().map_err(|n| n.wait_time_from(DefaultClock::default().now()))
    }

    pub fn check_refresh(&self, tenant_id: &str, refresh_token_hash_prefix: &str, per_min: u32) -> Result<(), Duration> {
        // hash prefix avoids storing raw token in memory key, but still buckets repeats
        let key = format!("{}:{}", tenant_id, refresh_token_hash_prefix);
        let lim = Self::limiter_for(&self.refresh, &key, per_min);
        lim.check().map_err(|n| n.wait_time_from(DefaultClock::default().now()))
    }
}
