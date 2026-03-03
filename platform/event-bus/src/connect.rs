//! NATS connection helper that extracts auth credentials from URLs.
//!
//! `async_nats::connect()` ignores userinfo in URLs. This helper parses
//! the URL, extracts user:password or token credentials, and connects
//! with proper `ConnectOptions`.

use async_nats::Client;

/// Connect to NATS, extracting any credentials embedded in the URL.
///
/// Supports:
/// - `nats://host:port` — no auth
/// - `nats://user:pass@host:port` — user/password auth
/// - `nats://token@host:port` — token auth (no colon in userinfo)
pub async fn connect_nats(url: &str) -> Result<Client, async_nats::ConnectError> {
    let (clean_url, auth) = parse_nats_auth(url);
    match auth {
        NatsAuth::None => async_nats::connect(&clean_url).await,
        NatsAuth::UserPass(user, pass) => {
            async_nats::ConnectOptions::with_user_and_password(user, pass)
                .connect(&clean_url)
                .await
        }
        NatsAuth::Token(token) => {
            async_nats::ConnectOptions::with_token(token)
                .connect(&clean_url)
                .await
        }
    }
}

enum NatsAuth {
    None,
    UserPass(String, String),
    Token(String),
}

/// Parse auth credentials from a NATS URL, returning (clean_url, auth).
fn parse_nats_auth(url: &str) -> (String, NatsAuth) {
    // Find the scheme separator
    let scheme_end = match url.find("://") {
        Some(pos) => pos + 3,
        None => return (url.to_string(), NatsAuth::None),
    };

    let rest = &url[scheme_end..];

    // Check for @ sign indicating userinfo
    let at_pos = match rest.find('@') {
        Some(pos) => pos,
        None => return (url.to_string(), NatsAuth::None),
    };

    let userinfo = &rest[..at_pos];
    let host_part = &rest[at_pos + 1..];
    let scheme = &url[..scheme_end];
    let clean_url = format!("{scheme}{host_part}");

    if let Some(colon_pos) = userinfo.find(':') {
        let user = userinfo[..colon_pos].to_string();
        let pass = userinfo[colon_pos + 1..].to_string();
        (clean_url, NatsAuth::UserPass(user, pass))
    } else {
        (clean_url, NatsAuth::Token(userinfo.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_no_auth() {
        let (url, auth) = parse_nats_auth("nats://host:4222");
        assert_eq!(url, "nats://host:4222");
        assert!(matches!(auth, NatsAuth::None));
    }

    #[test]
    fn parse_user_pass() {
        let (url, auth) = parse_nats_auth("nats://platform:secret@host:4222");
        assert_eq!(url, "nats://host:4222");
        match auth {
            NatsAuth::UserPass(u, p) => {
                assert_eq!(u, "platform");
                assert_eq!(p, "secret");
            }
            _ => panic!("expected UserPass"),
        }
    }

    #[test]
    fn parse_token() {
        let (url, auth) = parse_nats_auth("nats://mytoken@host:4222");
        assert_eq!(url, "nats://host:4222");
        match auth {
            NatsAuth::Token(t) => assert_eq!(t, "mytoken"),
            _ => panic!("expected Token"),
        }
    }
}
