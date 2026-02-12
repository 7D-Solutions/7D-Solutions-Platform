use axum::{extract::State, http::Method, routing::get, Json, Router};
use event_bus::{EventBus, InMemoryBus, NatsBus};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use tracing_subscriber::EnvFilter;

use ar_rs::{consumer_tasks, events::run_publisher_task, routes};

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let database_url =
        std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let host = std::env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
    let port: u16 = std::env::var("PORT")
        .unwrap_or_else(|_| "8086".to_string())
        .parse()
        .expect("PORT must be a valid u16");

    // Configure connection pool
    let is_test = std::env::var("NODE_ENV").unwrap_or_default() == "test";
    let max_connections = if is_test { 5 } else { 10 };
    let idle_timeout = if is_test {
        std::time::Duration::from_secs(60)
    } else {
        std::time::Duration::from_secs(300)
    };

    let db = PgPoolOptions::new()
        .max_connections(max_connections)
        .idle_timeout(Some(idle_timeout))
        .max_lifetime(Some(std::time::Duration::from_secs(1800)))
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect(&database_url)
        .await
        .expect("Failed to connect to Postgres");

    sqlx::migrate!("./db/migrations")
        .run(&db)
        .await
        .expect("Failed to run database migrations");

    tracing::info!("Database migrations applied successfully");

    // Initialize event bus based on BUS_TYPE environment variable
    let bus_type = std::env::var("BUS_TYPE").unwrap_or_else(|_| "inmemory".to_string());
    let event_bus: Arc<dyn EventBus> = match bus_type.to_lowercase().as_str() {
        "nats" => {
            let nats_url = std::env::var("NATS_URL")
                .unwrap_or_else(|_| "nats://localhost:4222".to_string());
            tracing::info!("Connecting to NATS at {}", nats_url);
            let nats_client = async_nats::connect(&nats_url)
                .await
                .expect("Failed to connect to NATS");
            Arc::new(NatsBus::new(nats_client))
        }
        "inmemory" | _ => {
            tracing::info!("Using in-memory event bus");
            Arc::new(InMemoryBus::new())
        }
    };

    // Spawn background publisher task
    let publisher_db = db.clone();
    let publisher_bus = event_bus.clone();
    tokio::spawn(async move {
        run_publisher_task(publisher_db, publisher_bus).await;
    });

    tracing::info!("Event publisher task started");

    // Spawn event consumer tasks
    consumer_tasks::start_payment_succeeded_consumer(event_bus.clone(), db.clone()).await;

    tracing::info!("Event consumer tasks started");

    // CORS configuration
    let cors = CorsLayer::new()
        .allow_origin([
            "http://localhost:5173".parse().unwrap(),
            "http://localhost:3000".parse().unwrap(),
            "http://localhost:3001".parse().unwrap(),
        ])
        .allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE])
        .allow_headers([
            axum::http::header::CONTENT_TYPE,
            axum::http::header::AUTHORIZATION,
            axum::http::HeaderName::from_static("idempotency-key"),
        ])
        .allow_credentials(true);

    let app = Router::new()
        .route("/api/health", get(health))
        .with_state(db.clone())
        .merge(routes::ar_router(db))
        .layer(cors)
        .into_make_service_with_connect_info::<SocketAddr>();

    let addr: SocketAddr = format!("{host}:{port}")
        .parse()
        .expect("Invalid HOST:PORT");

    tracing::info!("AR service listening on {addr}");

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("Failed to bind address");

    axum::serve(listener, app)
        .await
        .expect("Failed to start server");
}

async fn health(State(_db): State<PgPool>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "healthy",
        "service": "ar-rs"
    }))
}
