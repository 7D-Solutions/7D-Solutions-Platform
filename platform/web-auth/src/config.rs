use std::sync::Arc;

use security::claims::JwtVerifier;

/// Shared configuration for all WebAuthProxy handlers and the cookie middleware.
pub struct WebAuthConfig {
    pub cookie_prefix: String,
    pub refresh_cookie_path: String,
    pub access_max_age_secs: i64,
    pub refresh_max_age_secs: i64,
    pub secure: bool,
    pub auth_base_url: String,
    pub http_client: reqwest::Client,
    pub jwt_verifier: Option<Arc<JwtVerifier>>,
}

impl WebAuthConfig {
    pub fn access_cookie_name(&self) -> String {
        format!("{}_session", self.cookie_prefix)
    }

    pub fn refresh_cookie_name(&self) -> String {
        format!("{}_refresh", self.cookie_prefix)
    }

    pub fn build_access_set_cookie(&self, token: &str) -> String {
        let mut v = format!(
            "{}={}; Path=/; HttpOnly; SameSite=Lax; Max-Age={}",
            self.access_cookie_name(),
            token,
            self.access_max_age_secs,
        );
        if self.secure {
            v.push_str("; Secure");
        }
        v
    }

    pub fn build_refresh_set_cookie(&self, token: &str) -> String {
        let mut v = format!(
            "{}={}; Path={}; HttpOnly; SameSite=Lax; Max-Age={}",
            self.refresh_cookie_name(),
            token,
            self.refresh_cookie_path,
            self.refresh_max_age_secs,
        );
        if self.secure {
            v.push_str("; Secure");
        }
        v
    }

    pub fn build_access_clear_cookie(&self) -> String {
        let mut v = format!(
            "{}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0",
            self.access_cookie_name(),
        );
        if self.secure {
            v.push_str("; Secure");
        }
        v
    }

    pub fn build_refresh_clear_cookie(&self) -> String {
        let mut v = format!(
            "{}=; Path={}; HttpOnly; SameSite=Lax; Max-Age=0",
            self.refresh_cookie_name(),
            self.refresh_cookie_path,
        );
        if self.secure {
            v.push_str("; Secure");
        }
        v
    }
}

/// Parse a named cookie from a raw `Cookie:` header value.
pub fn read_cookie_from_header(raw: &str, name: &str) -> Option<String> {
    let prefix = format!("{}=", name);
    for pair in raw.split(';') {
        let pair = pair.trim();
        if let Some(value) = pair.strip_prefix(&prefix) {
            return Some(value.to_string());
        }
    }
    None
}

/// Extract a named cookie from request headers.
pub fn read_cookie(headers: &axum::http::HeaderMap, name: &str) -> Option<String> {
    let raw = headers.get(axum::http::header::COOKIE)?.to_str().ok()?;
    read_cookie_from_header(raw, name)
}
