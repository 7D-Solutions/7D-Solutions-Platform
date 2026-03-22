//! Party module seeder for demo-seed
//!
//! Creates aerospace customer and supplier companies via the Party service API.
//! Uses search-before-create to avoid duplicates since the Party module has no
//! idempotency mechanism or unique constraints on company names.

mod companies;
mod contacts;

use anyhow::Result;
use tracing::info;
use uuid::Uuid;

use crate::digest::DigestTracker;
use companies::{create_company, find_existing_party, COMPANIES};
use contacts::{add_address, add_contact, party_has_children};

// ---------------------------------------------------------------------------
// Party IDs returned for downstream modules
// ---------------------------------------------------------------------------

/// IDs of created parties, keyed by role
pub struct PartyIds {
    pub customers: Vec<(Uuid, String)>,
    pub suppliers: Vec<(Uuid, String)>,
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Seed all party data (customers + suppliers). Returns party IDs for downstream use.
pub async fn seed_parties(
    client: &reqwest::Client,
    party_url: &str,
    tracker: &mut DigestTracker,
) -> Result<PartyIds> {
    let mut ids = PartyIds {
        customers: Vec::new(),
        suppliers: Vec::new(),
    };

    for data in COMPANIES {
        // Search-before-create: check if company already exists
        let party_id = match find_existing_party(client, party_url, data.legal_name).await? {
            Some(existing_id) => {
                info!(
                    legal_name = data.legal_name,
                    party_id = %existing_id,
                    "Party already exists — skipping creation"
                );
                existing_id
            }
            None => {
                let new_id = create_company(client, party_url, data).await?;
                info!(
                    legal_name = data.legal_name,
                    party_id = %new_id,
                    role = data.role,
                    "Created party"
                );
                new_id
            }
        };

        // Check existing contacts/addresses before adding
        let (has_contacts, has_addresses) =
            party_has_children(client, party_url, party_id).await?;

        if !has_contacts {
            add_contact(client, party_url, party_id, data).await?;
            info!(party_id = %party_id, "Added primary contact");
        }

        if !has_addresses {
            add_address(client, party_url, party_id, data).await?;
            info!(party_id = %party_id, "Added registered address");
        }

        tracker.record_party(party_id, data.display_name, data.role);

        match data.role {
            "customer" => ids
                .customers
                .push((party_id, data.legal_name.to_string())),
            "supplier" => ids
                .suppliers
                .push((party_id, data.legal_name.to_string())),
            _ => {}
        }
    }

    Ok(ids)
}
