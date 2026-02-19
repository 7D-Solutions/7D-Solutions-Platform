pub mod ar;
pub mod auth;
pub mod party;

/// Standard platform headers injected on every outgoing request.
#[derive(Debug, Clone)]
pub struct PlatformHeaders {
    pub app_id: String,
    pub correlation_id: String,
    pub actor_id: String,
    pub authorization: Option<String>,
}

impl PlatformHeaders {
    pub fn new(app_id: impl Into<String>, correlation_id: impl Into<String>, actor_id: impl Into<String>) -> Self {
        Self {
            app_id: app_id.into(),
            correlation_id: correlation_id.into(),
            actor_id: actor_id.into(),
            authorization: None,
        }
    }

    pub fn with_auth(mut self, token: impl Into<String>) -> Self {
        self.authorization = Some(format!("Bearer {}", token.into()));
        self
    }
}
