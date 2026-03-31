use customer_portal::{
    auth::PortalJwt, build_router, config::Config, metrics::PortalMetrics, AppState,
};
use std::net::SocketAddr;
use std::sync::Arc;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let config = Config::from_env().unwrap_or_else(|err| {
        eprintln!("Configuration error: {}", err);
        std::process::exit(1);
    });

    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(10)
        .connect(&config.database_url)
        .await
        .expect("customer-portal: failed to connect to Postgres");

    sqlx::migrate!("./db/migrations")
        .run(&pool)
        .await
        .expect("customer-portal: failed to run migrations");

    let metrics = PortalMetrics::new().expect("customer-portal: metrics init failed");
    let portal_jwt = Arc::new(
        PortalJwt::new(
            &config.portal_jwt_private_key,
            &config.portal_jwt_public_key,
        )
        .expect("customer-portal: invalid portal JWT keys"),
    );

    let state = Arc::new(AppState {
        pool: pool.clone(),
        metrics,
        portal_jwt,
        config: config.clone(),
    });

    let app = build_router(state).into_make_service_with_connect_info::<SocketAddr>();
    let addr: SocketAddr = format!("{}:{}", config.host, config.port)
        .parse()
        .expect("Invalid HOST:PORT");

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("customer-portal: failed to bind address");

    tracing::info!("customer-portal listening on {}", addr);

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("customer-portal: server failed");

    pool.close().await;
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}
