//! Auto-wired platform service clients from manifest declarations.
//!
//! Modules declare which platform services they call in `module.toml`:
//!
//! ```toml
//! [platform.services]
//! party     = { enabled = true }
//! inventory = { enabled = true, timeout_secs = 60 }
//! ```
//!
//! At startup the SDK builds a [`PlatformClient`] per enabled service and
//! stores them in [`PlatformServices`]. Handlers retrieve typed clients via
//! [`ModuleContext::platform_client`].

use std::collections::HashMap;
use std::time::Duration;

use crate::http_client::{PlatformClient, TimeoutConfig};
use crate::manifest::{PlatformSection, ServiceEntry};
use crate::startup::StartupError;

/// Trait implemented by generated typed clients to declare their service name.
///
/// ```rust,ignore
/// impl PlatformService for PartiesClient {
///     const SERVICE_NAME: &'static str = "party";
///     fn from_platform_client(client: PlatformClient) -> Self {
///         Self::new(client)
///     }
/// }
///
/// // In a handler:
/// let party = ctx.platform_client::<PartiesClient>();
/// ```
pub trait PlatformService: Sized {
    /// Service name as declared in `[platform.services]` (e.g. `"party"`).
    const SERVICE_NAME: &'static str;

    /// Construct this typed client from a [`PlatformClient`].
    fn from_platform_client(client: PlatformClient) -> Self;
}

/// Pre-built `PlatformClient` instances keyed by service name.
///
/// Stored in [`ModuleContext`] extensions and used by
/// [`ModuleContext::platform_client`] to construct typed clients on demand.
#[derive(Debug)]
pub struct PlatformServices {
    clients: HashMap<String, PlatformClient>,
}

impl PlatformServices {
    /// Build clients from the manifest's `[platform.services]` section.
    ///
    /// For each enabled service, resolves the base URL from the env var
    /// `{SERVICE}_BASE_URL` (e.g. `PARTY_BASE_URL`), falling back to
    /// `default_url` if specified.
    pub fn from_manifest(
        platform: Option<&PlatformSection>,
        module_name: &str,
    ) -> Result<Self, StartupError> {
        let mut clients = HashMap::new();

        let services = match platform {
            Some(p) => &p.services,
            None => return Ok(Self { clients }),
        };

        // Obtain a service token for service-to-service auth.
        //
        // This startup token carries nil UUIDs for tenant_id/actor_id by design —
        // `get_service_token()` has no request context available at boot time.
        // It is stored ONLY as a last-resort fallback bearer token in each PlatformClient.
        //
        // inject_headers (http_client.rs) ALWAYS attempts to mint a per-request RSA JWT
        // via `mint_service_jwt_with_context(claims.tenant_id, claims.user_id)` before
        // falling back to this bearer token.  When `JWT_PRIVATE_KEY_PEM` is set in the
        // environment (required in production and staging), `mint_service_jwt_with_context`
        // will always succeed and the startup token is never used.
        //
        // INVARIANT: JWT_PRIVATE_KEY_PEM MUST be set in all non-local environments.
        // The tenant_context_canary_e2e test verifies this guarantee on every PR.
        let service_token = match security::get_service_token() {
            Ok(token) => {
                tracing::debug!(
                    module = %module_name,
                    "service token acquired — platform clients will authenticate"
                );
                Some(token)
            }
            Err(e) => {
                tracing::warn!(
                    module = %module_name,
                    error = %e,
                    "no service token available — platform clients will be unauthenticated"
                );
                None
            }
        };

        for (name, entry) in services {
            if !entry.enabled {
                tracing::debug!(
                    module = %module_name,
                    service = %name,
                    "platform service disabled — skipping"
                );
                continue;
            }

            let env_var = ServiceEntry::env_var_name(name);
            let base_url = match std::env::var(&env_var) {
                Ok(url) => url,
                Err(_) => match &entry.default_url {
                    Some(url) => {
                        tracing::info!(
                            module = %module_name,
                            service = %name,
                            env_var = %env_var,
                            default_url = %url,
                            "env var not set — using manifest default_url"
                        );
                        url.clone()
                    }
                    None => {
                        return Err(StartupError::Config(format!(
                            "platform service '{name}' requires env var {env_var} \
                             (or set default_url in [platform.services.{name}])"
                        )));
                    }
                },
            };

            let base_url = base_url.trim_end_matches('/').to_string();

            let timeout = match entry.timeout_secs {
                Some(secs) => TimeoutConfig {
                    request_timeout: Duration::from_secs(secs),
                    ..TimeoutConfig::default()
                },
                None => TimeoutConfig::default(),
            };

            let client = PlatformClient::with_timeout(base_url.clone(), timeout);
            let client = match &service_token {
                Some(token) => client.with_bearer_token(token.clone()),
                None => client,
            };

            tracing::info!(
                module = %module_name,
                service = %name,
                base_url = %base_url,
                "platform service client created"
            );

            clients.insert(name.clone(), client);
        }

        Ok(Self { clients })
    }

    /// Get the pre-built client for a service, if declared.
    pub fn get(&self, service_name: &str) -> Option<&PlatformClient> {
        self.clients.get(service_name)
    }

    /// Number of registered service clients.
    pub fn len(&self) -> usize {
        self.clients.len()
    }

    /// Whether no service clients are registered.
    pub fn is_empty(&self) -> bool {
        self.clients.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::PlatformSection;
    use std::collections::BTreeMap;

    fn entry(enabled: bool, default_url: Option<&str>, timeout_secs: Option<u64>) -> ServiceEntry {
        ServiceEntry {
            enabled,
            timeout_secs,
            default_url: default_url.map(String::from),
            extra: BTreeMap::new(),
        }
    }

    #[test]
    fn empty_manifest_returns_empty_services() {
        let svc = PlatformServices::from_manifest(None, "test").expect("test");
        assert!(svc.is_empty());
    }

    #[test]
    fn disabled_service_is_skipped() {
        let mut services = BTreeMap::new();
        services.insert("party".into(), entry(false, None, None));
        let section = PlatformSection {
            services,
            extra: BTreeMap::new(),
        };
        let svc = PlatformServices::from_manifest(Some(&section), "test").expect("test");
        assert!(svc.is_empty());
    }

    #[test]
    fn service_with_default_url_resolves() {
        let mut services = BTreeMap::new();
        services.insert(
            "party".into(),
            entry(true, Some("http://localhost:8098"), None),
        );
        let section = PlatformSection {
            services,
            extra: BTreeMap::new(),
        };

        // Ensure env var is NOT set
        std::env::remove_var("PARTY_BASE_URL");
        let svc = PlatformServices::from_manifest(Some(&section), "test").expect("test");
        assert!(svc.get("party").is_some());
    }

    #[test]
    fn env_var_overrides_default_url() {
        let mut services = BTreeMap::new();
        services.insert(
            "party".into(),
            entry(true, Some("http://localhost:8098"), None),
        );
        let section = PlatformSection {
            services,
            extra: BTreeMap::new(),
        };

        std::env::set_var("PARTY_BASE_URL", "http://custom:9999");
        let svc = PlatformServices::from_manifest(Some(&section), "test").expect("test");
        assert!(svc.get("party").is_some());
        std::env::remove_var("PARTY_BASE_URL");
    }

    #[test]
    fn missing_env_var_and_no_default_fails() {
        let mut services = BTreeMap::new();
        services.insert("mystery".into(), entry(true, None, None));
        let section = PlatformSection {
            services,
            extra: BTreeMap::new(),
        };

        std::env::remove_var("MYSTERY_BASE_URL");
        let err = PlatformServices::from_manifest(Some(&section), "test").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("MYSTERY_BASE_URL"), "got: {msg}");
    }
}
