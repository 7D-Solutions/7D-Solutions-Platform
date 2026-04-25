//! Verifies that the QBO client captures the intuit_tid response header in
//! structured log fields for both success and error responses.
//!
//! Run: ./scripts/cargo-slot.sh test -p integrations-rs --test qbo_intuit_tid_logged

use integrations_rs::domain::qbo::{client::QboClient, QboError, TokenProvider};
use std::sync::{Arc, Mutex};
use tracing::{
    field::{Field, Visit},
    Event, Subscriber,
};
use tracing_subscriber::{layer::Context, layer::SubscriberExt, util::SubscriberInitExt, Layer};

// ── Tracing field capture ─────────────────────────────────────────────────────

struct FieldCapture {
    entries: Arc<Mutex<Vec<(String, String)>>>,
}

impl<S: Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>> Layer<S>
    for FieldCapture
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut visitor = FieldVisitor(vec![]);
        event.record(&mut visitor);
        self.entries.lock().unwrap().extend(visitor.0);
    }
}

struct FieldVisitor(Vec<(String, String)>);

impl Visit for FieldVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.push((field.name().to_string(), value.to_string()));
    }
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.0.push((field.name().to_string(), format!("{:?}", value)));
    }
    fn record_u64(&mut self, field: &Field, value: u64) {
        self.0.push((field.name().to_string(), value.to_string()));
    }
    fn record_i64(&mut self, field: &Field, value: i64) {
        self.0.push((field.name().to_string(), value.to_string()));
    }
}

fn make_capture() -> (Arc<Mutex<Vec<(String, String)>>>, impl Layer<tracing_subscriber::Registry>) {
    let entries = Arc::new(Mutex::new(vec![]));
    let layer = FieldCapture {
        entries: entries.clone(),
    };
    (entries, layer)
}

fn has_intuit_tid(entries: &[(String, String)], expected_tid: &str) -> bool {
    entries
        .iter()
        .any(|(k, v)| k == "intuit_tid" && v == expected_tid)
}

// ── Token provider ────────────────────────────────────────────────────────────

struct FixedToken;

#[async_trait::async_trait]
impl TokenProvider for FixedToken {
    async fn get_token(&self) -> Result<String, QboError> {
        Ok("test-token".into())
    }
    async fn refresh_token(&self) -> Result<String, QboError> {
        Ok("test-token".into())
    }
}

fn make_client(base_url: &str) -> QboClient {
    QboClient::new(base_url, "realm-test", Arc::new(FixedToken))
}

// ── Local server helpers ──────────────────────────────────────────────────────

/// Start a local server that returns the given status code, body, and
/// `intuit_tid` response header on every request.
async fn start_server(status: u16, body: &'static str, tid: &'static str) -> String {
    let app = axum::Router::new().fallback(move || async move {
        (
            axum::http::StatusCode::from_u16(status).unwrap(),
            [(
                axum::http::header::HeaderName::from_static("intuit_tid"),
                axum::http::HeaderValue::from_static(tid),
            )],
            body,
        )
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{}/v3", addr)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// Successful QBO GET: intuit_tid must appear in a log field.
#[tokio::test]
async fn qbo_intuit_tid_logged() {
    const SUCCESS_TID: &str = "test-tid-success-123";
    const ERROR_TID: &str = "test-tid-error-456";

    // ── success path ──────────────────────────────────────────────────────────
    let success_body = r#"{"QueryResponse":{"Customer":[]}}"#;
    let success_url = start_server(200, success_body, SUCCESS_TID).await;

    let (success_entries, success_layer) = make_capture();
    let client = make_client(&success_url);
    {
        let _guard = tracing_subscriber::registry()
            .with(success_layer)
            .set_default();
        // get_entity delegates to get_with_refresh which logs intuit_tid
        let _ = client.get_entity("Customer", "1").await;
    }

    let success_fields = success_entries.lock().unwrap().clone();
    assert!(
        has_intuit_tid(&success_fields, SUCCESS_TID),
        "intuit_tid not found in success log fields; got: {:?}",
        success_fields
    );

    // ── error path ────────────────────────────────────────────────────────────
    let error_body = r#"{"Fault":{"Error":[{"Message":"Bad request","code":"400"}],"type":"ValidationFault"}}"#;
    let error_url = start_server(400, error_body, ERROR_TID).await;

    let (error_entries, error_layer) = make_capture();
    let error_client = make_client(&error_url);
    {
        let _guard = tracing_subscriber::registry()
            .with(error_layer)
            .set_default();
        let _ = error_client.get_entity("Customer", "1").await;
    }

    let error_fields = error_entries.lock().unwrap().clone();
    assert!(
        has_intuit_tid(&error_fields, ERROR_TID),
        "intuit_tid not found in error log fields; got: {:?}",
        error_fields
    );
}
