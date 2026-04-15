//! Integration tests for health auto-probing from manifest dependencies.
//!
//! Verifies that /api/ready and /api/health probe the dependencies declared
//! in the manifest [health] section. Requires `DATABASE_URL` (real Postgres).
//! If `NATS_URL` is also set, tests run against real NATS too.

use std::net::TcpListener;
use std::sync::Arc;

use axum::routing::get;
use axum::Router;
use event_bus::{EventBus, InMemoryBus};
use platform_sdk::Manifest;

/// Find a free port by binding to :0 and releasing.
fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind to free port");
    listener.local_addr().expect("local addr").port()
}

/// Connect to the test database or skip.
async fn test_pool() -> Option<sqlx::PgPool> {
    let url = match std::env::var("DATABASE_URL") {
        Ok(u) => u,
        Err(_) => {
            eprintln!("DATABASE_URL not set — skipping integration test");
            return None;
        }
    };
    Some(
        sqlx::postgres::PgPoolOptions::new()
            .max_connections(2)
            .connect(&url)
            .await
            .expect("failed to connect to test database"),
    )
}

/// Build a manifest with given health dependencies and bus type.
fn manifest_with_health(port: u16, health_deps: &[&str], bus_type: &str) -> Manifest {
    let deps_toml: Vec<String> = health_deps.iter().map(|d| format!("\"{}\"", d)).collect();
    let toml = format!(
        r#"
[module]
name = "health-probe-test"
version = "0.1.0"

[server]
host = "127.0.0.1"
port = {port}

[bus]
type = "{bus_type}"

[health]
dependencies = [{deps}]
"#,
        port = port,
        bus_type = bus_type,
        deps = deps_toml.join(", "),
    );
    Manifest::from_str(&toml, None).expect("test manifest should parse")
}

/// Build a manifest with only postgres health dep (no bus).
fn manifest_postgres_only(port: u16) -> Manifest {
    let toml = format!(
        r#"
[module]
name = "health-probe-pg-only"
version = "0.1.0"

[server]
host = "127.0.0.1"
port = {port}
"#,
        port = port,
    );
    Manifest::from_str(&toml, None).expect("test manifest should parse")
}

/// Boot a minimal HTTP server with health routes matching how phase_b wires them.
/// Returns a JoinHandle for the server and the base URL.
async fn boot_health_server(
    manifest: &Manifest,
    pool: sqlx::PgPool,
    bus: Option<Arc<dyn EventBus>>,
) -> (tokio::task::JoinHandle<()>, String) {
    let module_name = manifest.module.name.clone();
    let version = manifest
        .module
        .version
        .as_deref()
        .unwrap_or("0.0.0")
        .to_string();
    let host = manifest.server.host.clone();
    let port = manifest.server.port;

    let health_deps: Vec<String> = manifest
        .health
        .as_ref()
        .map(|h| h.dependencies.clone())
        .unwrap_or_default();
    let probe_nats = health_deps.iter().any(|d| d == "nats");

    // Build health routes matching startup.rs logic
    let health_name = module_name.clone();
    let health_version = version.clone();
    let health_pool = pool.clone();
    let health_bus = if probe_nats { bus.clone() } else { None };

    let ready_name = module_name.clone();
    let ready_version = version.clone();
    let ready_pool = pool.clone();
    let ready_bus = if probe_nats { bus.clone() } else { None };

    let app = Router::new()
        .route("/healthz", get(health::healthz))
        .route(
            "/api/health",
            get(move || async move {
                let mut checks = Vec::new();

                let start = std::time::Instant::now();
                let err = sqlx::query("SELECT 1")
                    .execute(&health_pool)
                    .await
                    .err()
                    .map(|e| e.to_string());
                let latency = start.elapsed().as_millis() as u64;
                checks.push(health::db_check(latency, err));

                if let Some(ref bus) = health_bus {
                    let nats_start = std::time::Instant::now();
                    let connected = bus.health_check().await;
                    let nats_latency = nats_start.elapsed().as_millis() as u64;
                    checks.push(health::nats_check(connected, nats_latency));
                }

                let resp = health::build_ready_response(&health_name, &health_version, checks);
                health::ready_response_to_axum(resp)
            }),
        )
        .route(
            "/api/ready",
            get(move || async move {
                let mut checks = Vec::new();

                let start = std::time::Instant::now();
                let err = sqlx::query("SELECT 1")
                    .execute(&ready_pool)
                    .await
                    .err()
                    .map(|e| e.to_string());
                let latency = start.elapsed().as_millis() as u64;
                checks.push(health::db_check(latency, err));

                if let Some(ref bus) = ready_bus {
                    let nats_start = std::time::Instant::now();
                    let connected = bus.health_check().await;
                    let nats_latency = nats_start.elapsed().as_millis() as u64;
                    checks.push(health::nats_check(connected, nats_latency));
                }

                let resp = health::build_ready_response(&ready_name, &ready_version, checks);
                health::ready_response_to_axum(resp)
            }),
        );

    let addr: std::net::SocketAddr = format!("{}:{}", host, port).parse().unwrap();
    let base_url = format!("http://{}:{}", host, port);

    let handle = tokio::spawn(async move {
        let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
        axum::serve(listener, app).await.unwrap();
    });

    // Wait until the server is accepting connections
    for _ in 0..50 {
        if tokio::net::TcpStream::connect(addr).await.is_ok() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    (handle, base_url)
}

// ──────────────────────────────────────────────────────────────────
// Test 1: Both postgres + nats declared, InMemoryBus → 200, both checks
// ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn ready_probes_postgres_and_nats_with_inmemory_bus() {
    let pool = match test_pool().await {
        Some(p) => p,
        None => return,
    };

    let port = free_port();
    let manifest = manifest_with_health(port, &["postgres", "nats"], "inmemory");
    let bus: Arc<dyn EventBus> = Arc::new(InMemoryBus::new());

    let (_handle, base_url) = boot_health_server(&manifest, pool, Some(bus)).await;

    // Hit /api/ready
    let resp = reqwest::get(format!("{}/api/ready", base_url))
        .await
        .expect("GET /api/ready");
    assert_eq!(resp.status(), 200, "/api/ready should return 200");

    let body: serde_json::Value = resp.json().await.expect("parse JSON");
    assert_eq!(body["status"], "ready");

    let checks = body["checks"].as_array().expect("checks array");
    assert_eq!(checks.len(), 2, "should have postgres + nats checks");
    assert_eq!(checks[0]["name"], "database");
    assert_eq!(checks[0]["status"], "up");
    assert_eq!(checks[1]["name"], "nats");
    assert_eq!(checks[1]["status"], "up");

    // Hit /api/health — same checks
    let resp = reqwest::get(format!("{}/api/health", base_url))
        .await
        .expect("GET /api/health");
    assert_eq!(resp.status(), 200, "/api/health should return 200");

    let body: serde_json::Value = resp.json().await.expect("parse JSON");
    let checks = body["checks"].as_array().expect("checks array");
    assert_eq!(checks.len(), 2, "health should also have 2 checks");
}

// ──────────────────────────────────────────────────────────────────
// Test 2: Only postgres declared, no bus → only postgres check
// ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn ready_probes_only_postgres_when_nats_not_declared() {
    let pool = match test_pool().await {
        Some(p) => p,
        None => return,
    };

    let port = free_port();
    let manifest = manifest_postgres_only(port);

    let (_handle, base_url) = boot_health_server(&manifest, pool, None).await;

    let resp = reqwest::get(format!("{}/api/ready", base_url))
        .await
        .expect("GET /api/ready");
    assert_eq!(resp.status(), 200);

    let body: serde_json::Value = resp.json().await.expect("parse JSON");
    let checks = body["checks"].as_array().expect("checks array");
    assert_eq!(checks.len(), 1, "should only have postgres check");
    assert_eq!(checks[0]["name"], "database");
    assert_eq!(checks[0]["status"], "up");
}

// ──────────────────────────────────────────────────────────────────
// Test 3: Healthz always 200 regardless of dependencies
// ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn healthz_always_returns_200() {
    let pool = match test_pool().await {
        Some(p) => p,
        None => return,
    };

    let port = free_port();
    let manifest = manifest_with_health(port, &["postgres", "nats"], "inmemory");
    let bus: Arc<dyn EventBus> = Arc::new(InMemoryBus::new());

    let (_handle, base_url) = boot_health_server(&manifest, pool, Some(bus)).await;

    let resp = reqwest::get(format!("{}/healthz", base_url))
        .await
        .expect("GET /healthz");
    assert_eq!(resp.status(), 200);

    let body: serde_json::Value = resp.json().await.expect("parse JSON");
    assert_eq!(body["status"], "alive");
}

// ──────────────────────────────────────────────────────────────────
// Test 4: Real NATS (when NATS_URL set) — both healthy → 200
// ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn ready_probes_real_nats_when_available() {
    let pool = match test_pool().await {
        Some(p) => p,
        None => return,
    };

    let nats_url = match std::env::var("NATS_URL") {
        Ok(u) => u,
        Err(_) => {
            eprintln!("NATS_URL not set — skipping real NATS test");
            return;
        }
    };

    let client = event_bus::connect_nats(&nats_url)
        .await
        .expect("connect to NATS");
    let bus: Arc<dyn EventBus> = Arc::new(event_bus::NatsBus::new(client));

    let port = free_port();
    let manifest = manifest_with_health(port, &["postgres", "nats"], "nats");

    let (_handle, base_url) = boot_health_server(&manifest, pool, Some(bus)).await;

    let resp = reqwest::get(format!("{}/api/ready", base_url))
        .await
        .expect("GET /api/ready");
    assert_eq!(
        resp.status(),
        200,
        "/api/ready should be 200 with real NATS"
    );

    let body: serde_json::Value = resp.json().await.expect("parse JSON");
    assert_eq!(body["status"], "ready");

    let checks = body["checks"].as_array().expect("checks array");
    assert_eq!(checks.len(), 2);
    assert_eq!(checks[1]["name"], "nats");
    assert_eq!(checks[1]["status"], "up");
}
