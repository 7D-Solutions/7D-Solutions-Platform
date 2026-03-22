//! Contact and address HTTP operations for party seeding

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::companies::CompanyData;

// ---------------------------------------------------------------------------
// API types
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct CreateContactRequest {
    first_name: String,
    last_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    phone: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    is_primary: Option<bool>,
}

#[derive(Serialize)]
struct CreateAddressRequest {
    line1: String,
    city: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    address_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    postal_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    country: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    is_primary: Option<bool>,
}

/// Full party view returned by GET /api/party/parties/{id}
#[derive(Debug, Deserialize)]
struct PartyView {
    contacts: Vec<serde_json::Value>,
    addresses: Vec<serde_json::Value>,
}

// ---------------------------------------------------------------------------
// HTTP operations
// ---------------------------------------------------------------------------

/// Check whether a party already has contacts and/or addresses.
pub(super) async fn party_has_children(
    client: &reqwest::Client,
    party_url: &str,
    party_id: Uuid,
) -> Result<(bool, bool)> {
    let url = format!("{}/api/party/parties/{}", party_url, party_id);

    let resp = client
        .get(&url)
        .send()
        .await
        .with_context(|| format!("GET /api/party/parties/{} network error", party_id))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        bail!(
            "GET /api/party/parties/{} failed {status}: {text}",
            party_id
        );
    }

    let view: PartyView = resp
        .json()
        .await
        .context("Failed to parse party view response")?;

    Ok((!view.contacts.is_empty(), !view.addresses.is_empty()))
}

pub(super) async fn add_contact(
    client: &reqwest::Client,
    party_url: &str,
    party_id: Uuid,
    data: &CompanyData,
) -> Result<()> {
    let url = format!("{}/api/party/parties/{}/contacts", party_url, party_id);

    let body = CreateContactRequest {
        first_name: data.contact_first.to_string(),
        last_name: data.contact_last.to_string(),
        email: Some(data.contact_email.to_string()),
        phone: None,
        role: Some(data.contact_role.to_string()),
        is_primary: Some(true),
    };

    let resp = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .with_context(|| format!("POST contacts for party {} network error", party_id))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        bail!(
            "POST contacts for party {} failed {status}: {text}",
            party_id
        );
    }

    Ok(())
}

pub(super) async fn add_address(
    client: &reqwest::Client,
    party_url: &str,
    party_id: Uuid,
    data: &CompanyData,
) -> Result<()> {
    let url = format!("{}/api/party/parties/{}/addresses", party_url, party_id);

    let body = CreateAddressRequest {
        line1: data.address_line1.to_string(),
        city: data.city.to_string(),
        address_type: Some("registered".to_string()),
        label: Some("Headquarters".to_string()),
        state: data.state.map(|s| s.to_string()),
        postal_code: Some(data.postal_code.to_string()),
        country: Some(data.country.to_string()),
        is_primary: Some(true),
    };

    let resp = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .with_context(|| format!("POST addresses for party {} network error", party_id))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        bail!(
            "POST addresses for party {} failed {status}: {text}",
            party_id
        );
    }

    Ok(())
}
