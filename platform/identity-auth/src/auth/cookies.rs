//! HttpOnly refresh-cookie helpers.
//!
//! The refresh cookie is:
//!   - `HttpOnly` (not readable by JavaScript)
//!   - `SameSite=Lax` (CSRF mitigation; refresh should never be invoked
//!     as a third-party sub-resource)
//!   - `Secure` (only in production / non-development environments)
//!   - `Path=/api/auth` (scoped so it's only sent to the auth service)
//!
//! Raw tokens are delivered only in this cookie; responses carry the access
//! token in the body and NO refresh token in the body for new cookie-flow
//! logins (see identity-auth spec).

use axum::http::HeaderMap;

/// Name of the refresh cookie. Keep in sync with identity-auth-sdk helpers.
pub const REFRESH_COOKIE_NAME: &str = "refresh";

/// Path the cookie is scoped to.
pub const REFRESH_COOKIE_PATH: &str = "/api/auth";

/// Parse the raw refresh token out of a `Cookie:` header, if present.
///
/// The header may carry multiple cookies (`a=1; refresh=xyz; b=2`); we extract
/// only the one named `REFRESH_COOKIE_NAME`.
pub fn read_refresh_cookie(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get(axum::http::header::COOKIE)?.to_str().ok()?;
    for pair in raw.split(';') {
        let pair = pair.trim();
        if let Some(value) = pair.strip_prefix(&format!("{REFRESH_COOKIE_NAME}=")) {
            return Some(value.to_string());
        }
    }
    None
}

/// Build a `Set-Cookie` value that plants the refresh cookie.
///
/// `max_age_seconds` is the cookie's advertised lifetime; set it to match the
/// current session's sliding expiry so browsers drop it when the session dies.
pub fn build_set_cookie(token: &str, max_age_seconds: i64, secure: bool) -> String {
    let mut v = format!(
        "{REFRESH_COOKIE_NAME}={token}; Path={REFRESH_COOKIE_PATH}; HttpOnly; SameSite=Lax; Max-Age={max_age_seconds}"
    );
    if secure {
        v.push_str("; Secure");
    }
    v
}

/// Build a `Set-Cookie` value that clears the refresh cookie (logout path).
pub fn build_clear_cookie(secure: bool) -> String {
    let mut v = format!(
        "{REFRESH_COOKIE_NAME}=; Path={REFRESH_COOKIE_PATH}; HttpOnly; SameSite=Lax; Max-Age=0"
    );
    if secure {
        v.push_str("; Secure");
    }
    v
}
