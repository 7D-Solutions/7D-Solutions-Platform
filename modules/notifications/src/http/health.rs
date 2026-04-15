use axum::{extract::State, http::StatusCode, Json};
use chrono::{DateTime, Utc};
use health::{
    build_ready_response, db_check_with_pool, ready_response_to_axum, PoolMetrics, ReadyResponse,
};
use sqlx::PgPool;
use std::{fs, io, path::PathBuf, time::Instant};

const STARTUP_HISTORY_ENV: &str = "NOTIFICATIONS_STARTUP_HISTORY_PATH";
const DEFAULT_STARTUP_HISTORY_PATH: &str = "/tmp/notifications-startup-history.log";
const CRASHLOOP_WINDOW_SECS: i64 = 120;
// Five recorded starts in the window correspond to four restarts after the
// initial boot, which is the crash-loop threshold we fail closed on.
const MAX_RECENT_STARTS: usize = 4;

/// Health check endpoint - returns basic service status (legacy, kept for compat)
pub async fn health() -> (StatusCode, Json<serde_json::Value>) {
    match startup_health_snapshot(Utc::now()) {
        Ok(snapshot) if snapshot.crashloop_detected => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({
                "status": "unhealthy",
                "service": "notifications-rs",
                "version": env!("CARGO_PKG_VERSION"),
                "reason": "recent startup surge",
                "recent_starts": snapshot.recent_starts,
                "window_seconds": CRASHLOOP_WINDOW_SECS,
            })),
        ),
        Ok(snapshot) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "healthy",
                "service": "notifications-rs",
                "version": env!("CARGO_PKG_VERSION"),
                "recent_starts": snapshot.recent_starts,
                "window_seconds": CRASHLOOP_WINDOW_SECS,
            })),
        ),
        Err(err) => {
            tracing::error!(error = %err, "notifications health snapshot failed");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({
                    "status": "unhealthy",
                    "service": "notifications-rs",
                    "version": env!("CARGO_PKG_VERSION"),
                    "reason": "health snapshot unavailable",
                })),
            )
        }
    }
}

/// GET /api/ready — readiness probe (verifies DB connectivity)
pub async fn ready(
    State(pool): State<PgPool>,
) -> Result<Json<ReadyResponse>, (StatusCode, Json<ReadyResponse>)> {
    let start = Instant::now();
    let db_err = crate::db::dlq_repo::ping(&pool)
        .await
        .err()
        .map(|e| e.to_string());
    let latency = start.elapsed().as_millis() as u64;

    let pool_metrics = PoolMetrics {
        size: pool.size(),
        idle: pool.num_idle() as u32,
        active: pool.size().saturating_sub(pool.num_idle() as u32),
    };

    let resp = build_ready_response(
        "notifications",
        env!("CARGO_PKG_VERSION"),
        vec![db_check_with_pool(latency, db_err, pool_metrics)],
    );
    ready_response_to_axum(resp)
}

/// Version endpoint - returns module identity and schema version
///
/// This endpoint provides build and deployment information:
/// - module_name: The service identifier
/// - module_version: Build version from Cargo.toml
/// - schema_version: Database schema version (latest migration)
///
/// Used for:
/// - Deployment verification
/// - Troubleshooting version mismatches
/// - Migration status checks
pub async fn version() -> Json<serde_json::Value> {
    // Schema version derived from latest migration timestamp
    // Format: YYYYMMDDNNNNNN (e.g., 20260216000001)
    const SCHEMA_VERSION: &str = "20260216000001";

    Json(serde_json::json!({
        "module_name": "notifications-rs",
        "module_version": env!("CARGO_PKG_VERSION"),
        "schema_version": SCHEMA_VERSION
    }))
}

/// Record that the notifications process started.
///
/// The health endpoint uses this history to detect crash-loops that would
/// otherwise appear healthy if the HTTP probe lands between restarts.
pub fn record_startup_event() {
    if let Err(err) = append_startup_event(Utc::now()) {
        tracing::warn!(error = %err, "failed to record notifications startup event");
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StartupHealthSnapshot {
    recent_starts: usize,
    crashloop_detected: bool,
}

fn startup_health_snapshot(now: DateTime<Utc>) -> io::Result<StartupHealthSnapshot> {
    let recent_starts = read_recent_startups(now)?.len();
    Ok(StartupHealthSnapshot {
        recent_starts,
        crashloop_detected: recent_starts > MAX_RECENT_STARTS,
    })
}

fn append_startup_event(now: DateTime<Utc>) -> io::Result<()> {
    let path = startup_history_path();
    let cutoff = now.timestamp() - CRASHLOOP_WINDOW_SECS;
    let mut events = read_recent_startups(now)?;
    events.retain(|ts| *ts >= cutoff);
    events.push(now.timestamp());
    write_startup_events(&path, &events)
}

fn read_recent_startups(now: DateTime<Utc>) -> io::Result<Vec<i64>> {
    let path = startup_history_path();
    let contents = match fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err),
    };

    let cutoff = now.timestamp() - CRASHLOOP_WINDOW_SECS;
    Ok(contents
        .lines()
        .filter_map(|line| line.trim().parse::<i64>().ok())
        .filter(|ts| *ts >= cutoff)
        .collect())
}

fn write_startup_events(path: &PathBuf, events: &[i64]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut buf = String::new();
    for event in events {
        buf.push_str(&event.to_string());
        buf.push('\n');
    }

    let tmp_path = path.with_extension("tmp");
    fs::write(&tmp_path, buf)?;
    fs::rename(&tmp_path, path)?;
    Ok(())
}

fn startup_history_path() -> PathBuf {
    std::env::var_os(STARTUP_HISTORY_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_STARTUP_HISTORY_PATH))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use uuid::Uuid;

    fn temp_history_path() -> PathBuf {
        std::env::temp_dir().join(format!("notifications-startup-{}.log", Uuid::new_v4()))
    }

    #[tokio::test]
    #[serial]
    async fn docker_health_crashloop() {
        let path = temp_history_path();
        std::env::set_var(STARTUP_HISTORY_ENV, &path);

        for _ in 0..4 {
            record_startup_event();
        }

        let (status, healthy_body) = health().await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(healthy_body["status"], "healthy");
        assert_eq!(healthy_body["recent_starts"], 4);

        record_startup_event();

        let (status, unhealthy_body) = health().await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(unhealthy_body["status"], "unhealthy");
        assert_eq!(unhealthy_body["reason"], "recent startup surge");
        assert_eq!(unhealthy_body["recent_starts"], 5);

        let _ = fs::remove_file(path);
        std::env::remove_var(STARTUP_HISTORY_ENV);
    }
}
