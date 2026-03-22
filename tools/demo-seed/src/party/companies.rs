//! Company data definitions and HTTP operations for party seeding

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// API types
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct CreateCompanyRequest {
    display_name: String,
    legal_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    tax_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    phone: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    website: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    address_line1: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    city: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    postal_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    country: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    metadata: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct CompanyResponse {
    id: Uuid,
}

/// Party search result item
#[derive(Debug, Deserialize)]
struct PartySearchItem {
    id: Uuid,
    legal_name: Option<String>,
}

/// Search response envelope
#[derive(Debug, Deserialize)]
struct SearchResponse {
    data: Vec<PartySearchItem>,
}

// ---------------------------------------------------------------------------
// Static seed data
// ---------------------------------------------------------------------------

pub(super) struct CompanyData {
    pub display_name: &'static str,
    pub legal_name: &'static str,
    pub tax_id: Option<&'static str>,
    pub city: &'static str,
    pub state: Option<&'static str>,
    pub postal_code: &'static str,
    pub country: &'static str,
    pub website: &'static str,
    pub role: &'static str,
    pub contact_first: &'static str,
    pub contact_last: &'static str,
    pub contact_email: &'static str,
    pub contact_role: &'static str,
    pub address_line1: &'static str,
}

pub(super) const COMPANIES: &[CompanyData] = &[
    // --- Customers ---
    CompanyData {
        display_name: "Boeing Defense, Space & Security",
        legal_name: "The Boeing Company",
        tax_id: Some("91-0425694"),
        city: "Seattle",
        state: Some("WA"),
        postal_code: "98108",
        country: "US",
        website: "https://www.boeing.com",
        role: "customer",
        contact_first: "Sarah",
        contact_last: "Mitchell",
        contact_email: "s.mitchell@boeing.example",
        contact_role: "Procurement Manager",
        address_line1: "100 N Riverside Plaza",
    },
    CompanyData {
        display_name: "Lockheed Martin Aeronautics",
        legal_name: "Lockheed Martin Corporation",
        tax_id: Some("52-1893632"),
        city: "Fort Worth",
        state: Some("TX"),
        postal_code: "76108",
        country: "US",
        website: "https://www.lockheedmartin.com",
        role: "customer",
        contact_first: "James",
        contact_last: "Patterson",
        contact_email: "j.patterson@lm.example",
        contact_role: "Supply Chain Director",
        address_line1: "1 Lockheed Blvd",
    },
    CompanyData {
        display_name: "Northrop Grumman",
        legal_name: "Northrop Grumman Corporation",
        tax_id: None,
        city: "Falls Church",
        state: Some("VA"),
        postal_code: "22042",
        country: "US",
        website: "https://www.northropgrumman.com",
        role: "customer",
        contact_first: "Linda",
        contact_last: "Chen",
        contact_email: "l.chen@ngc.example",
        contact_role: "Quality Assurance Lead",
        address_line1: "2980 Fairview Park Dr",
    },
    CompanyData {
        display_name: "Raytheon Technologies",
        legal_name: "RTX Corporation",
        tax_id: None,
        city: "Waltham",
        state: Some("MA"),
        postal_code: "02451",
        country: "US",
        website: "https://www.rtx.com",
        role: "customer",
        contact_first: "Michael",
        contact_last: "Torres",
        contact_email: "m.torres@rtx.example",
        contact_role: "Vendor Relations Manager",
        address_line1: "870 Winter St",
    },
    CompanyData {
        display_name: "General Dynamics",
        legal_name: "General Dynamics Corporation",
        tax_id: None,
        city: "Reston",
        state: Some("VA"),
        postal_code: "20191",
        country: "US",
        website: "https://www.gd.com",
        role: "customer",
        contact_first: "Karen",
        contact_last: "Walsh",
        contact_email: "k.walsh@gd.example",
        contact_role: "Program Manager",
        address_line1: "11011 Sunset Hills Rd",
    },
    // --- Suppliers ---
    CompanyData {
        display_name: "Bodycote",
        legal_name: "Bodycote plc",
        tax_id: None,
        city: "Macclesfield",
        state: None,
        postal_code: "SK10 2XB",
        country: "GB",
        website: "https://www.bodycote.com",
        role: "supplier",
        contact_first: "David",
        contact_last: "Barker",
        contact_email: "d.barker@bodycote.example",
        contact_role: "Account Manager",
        address_line1: "Springwood Court, Springwood Close",
    },
    CompanyData {
        display_name: "Alcoa",
        legal_name: "Alcoa Corporation",
        tax_id: None,
        city: "Pittsburgh",
        state: Some("PA"),
        postal_code: "15219",
        country: "US",
        website: "https://www.alcoa.com",
        role: "supplier",
        contact_first: "Robert",
        contact_last: "Jennings",
        contact_email: "r.jennings@alcoa.example",
        contact_role: "Sales Engineer",
        address_line1: "201 Isabella St",
    },
    CompanyData {
        display_name: "Carpenter Technology",
        legal_name: "Carpenter Technology Corporation",
        tax_id: None,
        city: "Philadelphia",
        state: Some("PA"),
        postal_code: "19103",
        country: "US",
        website: "https://www.carpentertechnology.com",
        role: "supplier",
        contact_first: "Angela",
        contact_last: "Price",
        contact_email: "a.price@cartech.example",
        contact_role: "Technical Sales Rep",
        address_line1: "1735 Market St",
    },
    CompanyData {
        display_name: "Precision Castparts",
        legal_name: "Precision Castparts Corp.",
        tax_id: None,
        city: "Portland",
        state: Some("OR"),
        postal_code: "97239",
        country: "US",
        website: "https://www.precast.com",
        role: "supplier",
        contact_first: "Steven",
        contact_last: "Hart",
        contact_email: "s.hart@pcc.example",
        contact_role: "Business Development Manager",
        address_line1: "4650 SW Macadam Ave",
    },
    CompanyData {
        display_name: "Hexcel",
        legal_name: "Hexcel Corporation",
        tax_id: None,
        city: "Stamford",
        state: Some("CT"),
        postal_code: "06901",
        country: "US",
        website: "https://www.hexcel.com",
        role: "supplier",
        contact_first: "Patricia",
        contact_last: "Dunn",
        contact_email: "p.dunn@hexcel.example",
        contact_role: "Materials Specialist",
        address_line1: "281 Tresser Blvd",
    },
];

// ---------------------------------------------------------------------------
// Search-before-create
// ---------------------------------------------------------------------------

/// Search for an existing party by exact legal_name match.
pub(super) async fn find_existing_party(
    client: &reqwest::Client,
    party_url: &str,
    legal_name: &str,
) -> Result<Option<Uuid>> {
    let url = format!("{}/api/party/parties/search", party_url);

    let resp = client
        .get(&url)
        .query(&[("name", legal_name), ("limit", "10")])
        .send()
        .await
        .with_context(|| {
            format!(
                "GET /api/party/parties/search for '{}' network error",
                legal_name
            )
        })?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        bail!("GET /api/party/parties/search failed {status}: {text}");
    }

    let search: SearchResponse = resp
        .json()
        .await
        .context("Failed to parse party search response")?;

    // Exact match on legal_name to avoid false positives
    for item in &search.data {
        if item.legal_name.as_deref() == Some(legal_name) {
            return Ok(Some(item.id));
        }
    }

    Ok(None)
}

// ---------------------------------------------------------------------------
// Create operations
// ---------------------------------------------------------------------------

pub(super) async fn create_company(
    client: &reqwest::Client,
    party_url: &str,
    data: &CompanyData,
) -> Result<Uuid> {
    let url = format!("{}/api/party/companies", party_url);

    let body = CreateCompanyRequest {
        display_name: data.display_name.to_string(),
        legal_name: data.legal_name.to_string(),
        tax_id: data.tax_id.map(|s| s.to_string()),
        email: None,
        phone: None,
        website: Some(data.website.to_string()),
        address_line1: Some(data.address_line1.to_string()),
        city: Some(data.city.to_string()),
        state: data.state.map(|s| s.to_string()),
        postal_code: Some(data.postal_code.to_string()),
        country: Some(data.country.to_string()),
        metadata: Some(serde_json::json!({
            "tags": [data.role],
            "demo": true,
        })),
    };

    let resp = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .context("POST /api/party/companies network error")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        bail!("POST /api/party/companies failed {status}: {text}");
    }

    let company: CompanyResponse = resp
        .json()
        .await
        .context("Failed to parse company response")?;

    Ok(company.id)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ten_companies_defined() {
        assert_eq!(
            COMPANIES.len(),
            10,
            "Expected 10 companies (5 customers + 5 suppliers)"
        );
    }

    #[test]
    fn five_customers_five_suppliers() {
        let customers: Vec<_> = COMPANIES.iter().filter(|c| c.role == "customer").collect();
        let suppliers: Vec<_> = COMPANIES.iter().filter(|c| c.role == "supplier").collect();
        assert_eq!(customers.len(), 5, "Expected 5 customers");
        assert_eq!(suppliers.len(), 5, "Expected 5 suppliers");
    }

    #[test]
    fn legal_names_are_unique() {
        let mut names: Vec<&str> = COMPANIES.iter().map(|c| c.legal_name).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), COMPANIES.len(), "Duplicate legal names found");
    }

    #[test]
    fn all_companies_have_required_fields() {
        for c in COMPANIES {
            assert!(!c.display_name.is_empty(), "Empty display_name");
            assert!(!c.legal_name.is_empty(), "Empty legal_name");
            assert!(!c.city.is_empty(), "Empty city for {}", c.legal_name);
            assert!(
                !c.postal_code.is_empty(),
                "Empty postal_code for {}",
                c.legal_name
            );
            assert!(!c.country.is_empty(), "Empty country for {}", c.legal_name);
            assert!(
                !c.contact_first.is_empty(),
                "Empty contact_first for {}",
                c.legal_name
            );
            assert!(
                !c.contact_last.is_empty(),
                "Empty contact_last for {}",
                c.legal_name
            );
            assert!(
                !c.contact_email.is_empty(),
                "Empty contact_email for {}",
                c.legal_name
            );
            assert!(
                !c.address_line1.is_empty(),
                "Empty address_line1 for {}",
                c.legal_name
            );
        }
    }

    #[test]
    fn boeing_has_correct_tax_id() {
        let boeing = COMPANIES
            .iter()
            .find(|c| c.legal_name == "The Boeing Company")
            .unwrap();
        assert_eq!(boeing.tax_id, Some("91-0425694"));
        assert_eq!(boeing.role, "customer");
    }

    #[test]
    fn lockheed_has_correct_tax_id() {
        let lm = COMPANIES
            .iter()
            .find(|c| c.legal_name == "Lockheed Martin Corporation")
            .unwrap();
        assert_eq!(lm.tax_id, Some("52-1893632"));
    }

    #[test]
    fn bodycote_is_uk_supplier() {
        let bc = COMPANIES
            .iter()
            .find(|c| c.legal_name == "Bodycote plc")
            .unwrap();
        assert_eq!(bc.country, "GB");
        assert_eq!(bc.role, "supplier");
        assert!(bc.state.is_none(), "UK company should have no US state");
    }
}
