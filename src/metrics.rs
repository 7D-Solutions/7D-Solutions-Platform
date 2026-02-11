use prometheus::{
    Encoder, HistogramOpts, HistogramVec, IntCounterVec, IntGaugeVec, Opts, Registry, TextEncoder,
};
use std::time::Instant;

#[derive(Clone)]
pub struct Metrics {
    registry: Registry,

    // Counters
    pub auth_login_total: IntCounterVec,
    pub auth_register_total: IntCounterVec,
    pub auth_refresh_total: IntCounterVec,
    pub auth_logout_total: IntCounterVec,
    pub auth_rate_limited_total: IntCounterVec,
    pub auth_nats_publish_fail_total: IntCounterVec,
    pub auth_refresh_replay_total: IntCounterVec,

    // Histograms
    pub http_request_duration_seconds: HistogramVec,
    pub auth_password_verify_duration_seconds: HistogramVec,

    // Dependency gauges
    pub dep_up: IntGaugeVec,
}

impl Metrics {
    pub fn new() -> Self {
        let registry = Registry::new();

        let auth_login_total = IntCounterVec::new(
            Opts::new("auth_login_total", "Total login attempts"),
            &["result", "reason"], // result: success|failure, reason: invalid|locked|not_found|inactive|rate_limited|error
        )
        .expect("metric");

        let auth_register_total = IntCounterVec::new(
            Opts::new("auth_register_total", "Total register attempts"),
            &["result", "reason"],
        )
        .expect("metric");

        let auth_refresh_total = IntCounterVec::new(
            Opts::new("auth_refresh_total", "Total refresh attempts"),
            &["result", "reason"], // invalid|revoked|expired|rate_limited|error
        )
        .expect("metric");

        let auth_logout_total = IntCounterVec::new(
            Opts::new("auth_logout_total", "Total logout attempts"),
            &["result", "reason"],
        )
        .expect("metric");

        let auth_rate_limited_total = IntCounterVec::new(
            Opts::new("auth_rate_limited_total", "Requests rate limited"),
            &["scope"], // ip|email|refresh
        )
        .expect("metric");

        let auth_nats_publish_fail_total = IntCounterVec::new(
            Opts::new("auth_nats_publish_fail_total", "NATS publish failures"),
            &["event_type"],
        )
        .expect("metric");

        let auth_refresh_replay_total = IntCounterVec::new(
            Opts::new("auth_refresh_replay_total", "Refresh replay attempts (revoked token reuse)"),
            &["tenant_id"],
        )
        .expect("metric");

        let http_request_duration_seconds = HistogramVec::new(
            HistogramOpts::new("http_request_duration_seconds", "HTTP request duration seconds"),
            &["path", "method", "status"],
        )
        .expect("metric");

        let auth_password_verify_duration_seconds = HistogramVec::new(
            HistogramOpts::new(
                "auth_password_verify_duration_seconds",
                "Argon2 password verify duration seconds",
            ),
            &["result"], // ok|fail|error
        )
        .expect("metric");

        let dep_up = IntGaugeVec::new(
            Opts::new("auth_dependency_up", "Dependency up gauge"),
            &["dep"], // db|nats|ready
        )
        .expect("metric");

        registry
            .register(Box::new(auth_login_total.clone()))
            .unwrap();
        registry
            .register(Box::new(auth_register_total.clone()))
            .unwrap();
        registry
            .register(Box::new(auth_refresh_total.clone()))
            .unwrap();
        registry
            .register(Box::new(auth_logout_total.clone()))
            .unwrap();
        registry
            .register(Box::new(auth_rate_limited_total.clone()))
            .unwrap();
        registry
            .register(Box::new(auth_nats_publish_fail_total.clone()))
            .unwrap();
        registry
            .register(Box::new(auth_refresh_replay_total.clone()))
            .unwrap();
        registry
            .register(Box::new(http_request_duration_seconds.clone()))
            .unwrap();
        registry
            .register(Box::new(auth_password_verify_duration_seconds.clone()))
            .unwrap();
        registry.register(Box::new(dep_up.clone())).unwrap();

        Self {
            registry,
            auth_login_total,
            auth_register_total,
            auth_refresh_total,
            auth_logout_total,
            auth_rate_limited_total,
            auth_nats_publish_fail_total,
            auth_refresh_replay_total,
            http_request_duration_seconds,
            auth_password_verify_duration_seconds,
            dep_up,
        }
    }

    pub fn render(&self) -> Result<String, String> {
        let encoder = TextEncoder::new();
        let mf = self.registry.gather();
        let mut buf = Vec::new();
        encoder
            .encode(&mf, &mut buf)
            .map_err(|e| e.to_string())?;
        String::from_utf8(buf).map_err(|e| e.to_string())
    }

    pub fn timer() -> Instant {
        Instant::now()
    }
}
