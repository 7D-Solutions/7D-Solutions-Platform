//! Startup env validation for OAuth redirect security vars.
//! Exposed as a public function so integration tests can test the panic paths
//! without going through main.rs.

/// Validate OAuth state-signing and redirect env vars.
///
/// Fatal if OAUTH_STATE_SECRET is missing or shorter than 32 bytes,
/// OAUTH_DEFAULT_RETURN_URL is missing, or OAUTH_ALLOWED_RETURN_ORIGINS is empty.
pub fn validate_oauth_env_pub() {
    let secret = std::env::var("OAUTH_STATE_SECRET").unwrap_or_default();
    if secret.is_empty() {
        panic!(
            "Startup validation failed: OAUTH_STATE_SECRET is not set. \
             This key signs OAuth state parameters and is required to prevent CSRF attacks. \
             Set it to a random string of at least 32 characters."
        );
    }
    if secret.len() < 32 {
        panic!(
            "Startup validation failed: OAUTH_STATE_SECRET is too short ({} chars); \
             at least 32 characters are required for adequate HMAC entropy.",
            secret.len()
        );
    }
    let default_return = std::env::var("OAUTH_DEFAULT_RETURN_URL").unwrap_or_default();
    if default_return.is_empty() {
        panic!(
            "Startup validation failed: OAUTH_DEFAULT_RETURN_URL is not set. \
             This URL is used as the fallback redirect destination when no return_url is provided."
        );
    }
    let allowed_origins = std::env::var("OAUTH_ALLOWED_RETURN_ORIGINS").unwrap_or_default();
    if allowed_origins.is_empty() {
        panic!(
            "Startup validation failed: OAUTH_ALLOWED_RETURN_ORIGINS is not set or empty. \
             This comma-separated list of origins restricts which domains the OAuth callback \
             may redirect to, preventing open-redirect attacks."
        );
    }
}
