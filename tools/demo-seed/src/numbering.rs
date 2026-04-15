//! Numbering policy seeder for demo-seed
//!
//! Creates numbering policies via PUT /policies/{entity} on the numbering service.
//! PUT is natively idempotent (upsert semantics via ON CONFLICT UPDATE).

use anyhow::{bail, Context, Result};
use serde::Serialize;
use tracing::info;

use crate::digest::DigestTracker;

#[derive(Serialize)]
struct PolicyRequest {
    pattern: String,
    prefix: String,
    padding: i32,
}

struct PolicyConfig {
    entity: &'static str,
    pattern: &'static str,
    prefix: &'static str,
    padding: i32,
}

const POLICIES: &[PolicyConfig] = &[
    PolicyConfig {
        entity: "purchase-order",
        pattern: "PO-{YYYY}-{number}",
        prefix: "PO",
        padding: 5,
    },
    PolicyConfig {
        entity: "sales-order",
        pattern: "SO-{YYYY}-{number}",
        prefix: "SO",
        padding: 5,
    },
    PolicyConfig {
        entity: "work-order",
        pattern: "WO-{YYYY}-{number}",
        prefix: "WO",
        padding: 5,
    },
    PolicyConfig {
        entity: "eco",
        pattern: "ECO-{number}",
        prefix: "ECO",
        padding: 4,
    },
    PolicyConfig {
        entity: "shipment",
        pattern: "SHP-{YYYY}{MM}-{number}",
        prefix: "SHP",
        padding: 4,
    },
    PolicyConfig {
        entity: "invoice",
        pattern: "INV-{number}",
        prefix: "INV",
        padding: 6,
    },
    PolicyConfig {
        entity: "bom",
        pattern: "BOM-{number}",
        prefix: "BOM",
        padding: 4,
    },
    PolicyConfig {
        entity: "receiving-report",
        pattern: "RR-{YYYY}{MM}-{number}",
        prefix: "RR",
        padding: 4,
    },
];

/// Seed all numbering policies. Returns the entity names of policies created.
pub async fn seed_numbering_policies(
    client: &reqwest::Client,
    numbering_url: &str,
    tracker: &mut DigestTracker,
) -> Result<Vec<String>> {
    let mut entities = Vec::with_capacity(POLICIES.len());

    for policy in POLICIES {
        let url = format!("{}/policies/{}", numbering_url, policy.entity);

        let body = PolicyRequest {
            pattern: policy.pattern.to_string(),
            prefix: policy.prefix.to_string(),
            padding: policy.padding,
        };

        let resp = client
            .put(&url)
            .json(&body)
            .send()
            .await
            .with_context(|| format!("PUT /policies/{} network error", policy.entity))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            bail!("PUT /policies/{} failed {status}: {text}", policy.entity);
        }

        tracker.record_numbering_policy(policy.entity, policy.prefix);
        info!(
            entity = policy.entity,
            prefix = policy.prefix,
            "Created numbering policy"
        );
        entities.push(policy.entity.to_string());
    }

    Ok(entities)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_eight_policies_defined() {
        assert_eq!(POLICIES.len(), 8, "Expected 8 numbering policies");
    }

    #[test]
    fn all_patterns_contain_number_token() {
        for p in POLICIES {
            assert!(
                p.pattern.contains("{number}"),
                "Policy {} pattern '{}' missing {{number}} token",
                p.entity,
                p.pattern
            );
        }
    }

    #[test]
    fn padding_values_in_range() {
        for p in POLICIES {
            assert!(
                p.padding >= 0 && p.padding <= 20,
                "Policy {} padding {} out of range 0-20",
                p.entity,
                p.padding
            );
        }
    }

    #[test]
    fn policy_entities_are_unique() {
        let mut entities: Vec<&str> = POLICIES.iter().map(|p| p.entity).collect();
        entities.sort();
        entities.dedup();
        assert_eq!(
            entities.len(),
            POLICIES.len(),
            "Duplicate entity names found"
        );
    }
}
