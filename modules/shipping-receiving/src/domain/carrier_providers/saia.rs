//! Saia LTL carrier provider.
//!
//! Implements `CarrierProvider` for Saia via their REST API:
//! - Rate quotes:     POST /rating/v2/quote
//! - Bill of Lading:  POST /shipments/v2/bol
//! - Tracking:        GET  /tracking/v2/{pro_number}
//! - Void:            POST /shipments/v2/{pro_number}/void
//!
//! ## Config JSON
//! ```json
//! {
//!   "api_key":        "your-saia-api-key",
//!   "account_number": "your-saia-account-number",
//!   "base_url":       "https://api.saiasecure.com"
//! }
//! ```
//! `base_url` is optional — defaults to the Saia production endpoint.
//! Use `https://api.saiasecure.com/webservice-sandbox` for sandbox testing.
//!
//! ## Auth
//! Both `api_key` and `account_number` are required. The API key is sent as
//! the username in HTTP Basic auth (`Authorization: Basic base64(api_key:)`
//! with empty password); the account number is sent in `X-Saia-Account-Number`.
//!
//! ## PRO number format
//! Saia PRO numbers are 9-digit numeric strings. Do not assume this format is
//! shared with other LTL carriers (R&L, XPO, ODFL all differ).

use async_trait::async_trait;
use reqwest::Client;
use serde_json::Value;

use super::{
    CarrierProvider, CarrierProviderError, LabelResult, RateQuote, TrackingEvent, TrackingResult,
};

const SAIA_DEFAULT_URL: &str = "https://api.saiasecure.com";

pub struct SaiaCarrierProvider;

// ── Config helpers ─────────────────────────────────────────────

fn get_api_key(config: &Value) -> Result<&str, CarrierProviderError> {
    config["api_key"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            CarrierProviderError::CredentialsError(
                "Saia config missing required field 'api_key'".to_string(),
            )
        })
}

fn get_account_number(config: &Value) -> Result<&str, CarrierProviderError> {
    config["account_number"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            CarrierProviderError::CredentialsError(
                "Saia config missing required field 'account_number'".to_string(),
            )
        })
}

fn get_base_url(config: &Value) -> &str {
    config["base_url"]
        .as_str()
        .filter(|s| !s.is_empty())
        .unwrap_or(SAIA_DEFAULT_URL)
}

// ── Basic auth encoding ────────────────────────────────────────

// Saia uses HTTP Basic auth with the API key as the username and empty password.
pub(crate) fn basic_auth_header(api_key: &str) -> String {
    let credentials = format!("{api_key}:");
    format!("Basic {}", base64_encode(credentials.as_bytes()))
}

fn base64_encode(input: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((input.len() + 2) / 3 * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let combined = (b0 << 16) | (b1 << 8) | b2;
        out.push(CHARS[((combined >> 18) & 0x3F) as usize] as char);
        out.push(CHARS[((combined >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            out.push(CHARS[((combined >> 6) & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(CHARS[(combined & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

// ── HTTP helpers ───────────────────────────────────────────────

async fn saia_post(
    client: &Client,
    api_key: &str,
    account_number: &str,
    url: &str,
    body: &Value,
) -> Result<Value, CarrierProviderError> {
    let resp = client
        .post(url)
        .header("Authorization", basic_auth_header(api_key))
        .header("X-Saia-Account-Number", account_number)
        .header("Content-Type", "application/json")
        .json(body)
        .send()
        .await
        .map_err(|e| CarrierProviderError::ApiError(format!("Saia HTTP error: {e}")))?;

    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| CarrierProviderError::ApiError(format!("Saia response read error: {e}")))?;

    let json: Value = serde_json::from_str(&text).unwrap_or_else(|_| serde_json::json!({}));

    if !status.is_success() {
        let msg = extract_saia_error(&json, status.as_u16());
        return Err(CarrierProviderError::ApiError(format!(
            "Saia API error (HTTP {status}): {msg}"
        )));
    }

    Ok(json)
}

async fn saia_get(
    client: &Client,
    api_key: &str,
    account_number: &str,
    url: &str,
) -> Result<Value, CarrierProviderError> {
    let resp = client
        .get(url)
        .header("Authorization", basic_auth_header(api_key))
        .header("X-Saia-Account-Number", account_number)
        .send()
        .await
        .map_err(|e| CarrierProviderError::ApiError(format!("Saia HTTP error: {e}")))?;

    let status = resp.status();

    if status.as_u16() == 404 {
        return Err(CarrierProviderError::NotFound(
            "Saia: PRO number not found".to_string(),
        ));
    }

    let text = resp
        .text()
        .await
        .map_err(|e| CarrierProviderError::ApiError(format!("Saia response read error: {e}")))?;

    let json: Value = serde_json::from_str(&text).unwrap_or_else(|_| serde_json::json!({}));

    if !status.is_success() {
        let msg = extract_saia_error(&json, status.as_u16());
        return Err(CarrierProviderError::ApiError(format!(
            "Saia API error (HTTP {status}): {msg}"
        )));
    }

    Ok(json)
}

// ── Error extraction ───────────────────────────────────────────

pub(crate) fn extract_saia_error(json: &Value, _status: u16) -> String {
    if let Some(msg) = json["message"].as_str().filter(|s| !s.is_empty()) {
        return msg.to_string();
    }
    if let Some(err) = json["error"].as_str().filter(|s| !s.is_empty()) {
        return err.to_string();
    }
    if let Some(desc) = json["errorMessage"].as_str().filter(|s| !s.is_empty()) {
        return desc.to_string();
    }
    "unknown Saia API error".to_string()
}

// ── Rate request builder ───────────────────────────────────────

pub(crate) fn build_rate_request(req: &Value, account_number: &str) -> Value {
    let origin_zip = req["origin_zip"].as_str().unwrap_or("30301");
    let origin_city = req["origin_city"].as_str().unwrap_or("Atlanta");
    let origin_state = req["origin_state"].as_str().unwrap_or("GA");
    let dest_zip = req["dest_zip"].as_str().unwrap_or("94105");
    let dest_city = req["dest_city"].as_str().unwrap_or("San Francisco");
    let dest_state = req["dest_state"].as_str().unwrap_or("CA");
    let weight_lbs = req["weight_lbs"].as_f64().unwrap_or(500.0);
    let freight_class = req["freight_class"].as_str().unwrap_or("70");
    let pieces = req["pieces"].as_u64().unwrap_or(1);
    let description = req["description"].as_str().unwrap_or("LTL Freight");

    serde_json::json!({
        "accountNumber": account_number,
        "origin": {
            "postalCode": origin_zip,
            "city": origin_city,
            "stateCode": origin_state
        },
        "destination": {
            "postalCode": dest_zip,
            "city": dest_city,
            "stateCode": dest_state
        },
        "commodities": [
            {
                "weight": weight_lbs,
                "freightClass": freight_class,
                "pieces": pieces,
                "description": description
            }
        ]
    })
}

// ── Rate response parser ───────────────────────────────────────
//
// Saia /rating/v2/quote may return a `services` array or a flat object with
// `total_charge`, `transit_days`, `service_level` fields. Both shapes handled;
// the flat shape is wrapped into a single-element vec.

pub(crate) fn parse_rate_response(json: &Value) -> Result<Vec<RateQuote>, CarrierProviderError> {
    if let Some(services) = json["services"].as_array() {
        if services.is_empty() {
            return Err(CarrierProviderError::ApiError(
                "Saia rate response returned no service options".to_string(),
            ));
        }
        let quotes = services
            .iter()
            .map(|s| {
                let service_level = s["service_level"]
                    .as_str()
                    .or_else(|| s["serviceLevel"].as_str())
                    .unwrap_or("LTL Standard")
                    .to_string();
                let total_charge = s["total_charge"]
                    .as_f64()
                    .or_else(|| s["totalCharge"].as_f64())
                    .unwrap_or(0.0);
                let total_charge_minor = (total_charge * 100.0).round() as i64;
                let estimated_days = s["transit_days"]
                    .as_i64()
                    .or_else(|| s["transitDays"].as_i64())
                    .map(|d| d as i32);
                RateQuote {
                    service_level,
                    carrier_code: "saia".to_string(),
                    total_charge_minor,
                    currency: "USD".to_string(),
                    estimated_days,
                }
            })
            .collect();
        return Ok(quotes);
    }

    // Flat single-rate response shape
    let service_level = json["service_level"]
        .as_str()
        .unwrap_or("LTL Standard")
        .to_string();
    let total_charge = json["total_charge"].as_f64().unwrap_or(0.0);
    let total_charge_minor = (total_charge * 100.0).round() as i64;
    let estimated_days = json["transit_days"].as_i64().map(|d| d as i32);

    if total_charge_minor == 0 && json["service_level"].is_null() {
        return Err(CarrierProviderError::ApiError(
            "Saia rate response missing 'services' array and flat rate fields".to_string(),
        ));
    }

    Ok(vec![RateQuote {
        service_level,
        carrier_code: "saia".to_string(),
        total_charge_minor,
        currency: "USD".to_string(),
        estimated_days,
    }])
}

// ── BOL request builder ────────────────────────────────────────

pub(crate) fn build_bol_request(req: &Value, account_number: &str) -> Value {
    let from_name = req["from_name"].as_str().unwrap_or("Shipper");
    let from_address = req["from_address"].as_str().unwrap_or("123 Main St");
    let from_city = req["from_city"].as_str().unwrap_or("Atlanta");
    let from_state = req["from_state"].as_str().unwrap_or("GA");
    let from_zip = req["from_zip"].as_str().unwrap_or("30301");
    let to_name = req["to_name"].as_str().unwrap_or("Consignee");
    let to_address = req["to_address"].as_str().unwrap_or("456 Market St");
    let to_city = req["to_city"].as_str().unwrap_or("San Francisco");
    let to_state = req["to_state"].as_str().unwrap_or("CA");
    let to_zip = req["to_zip"].as_str().unwrap_or("94105");
    let weight_lbs = req["weight_lbs"].as_f64().unwrap_or(500.0);
    let freight_class = req["freight_class"].as_str().unwrap_or("70");
    let pieces = req["pieces"].as_u64().unwrap_or(1);
    let description = req["description"].as_str().unwrap_or("LTL Freight");
    let special_instructions = req["special_instructions"].as_str().unwrap_or("");

    serde_json::json!({
        "accountNumber": account_number,
        "shipper": {
            "name": from_name,
            "address": from_address,
            "city": from_city,
            "stateCode": from_state,
            "postalCode": from_zip
        },
        "consignee": {
            "name": to_name,
            "address": to_address,
            "city": to_city,
            "stateCode": to_state,
            "postalCode": to_zip
        },
        "commodities": [
            {
                "weight": weight_lbs,
                "freightClass": freight_class,
                "pieces": pieces,
                "description": description
            }
        ],
        "specialInstructions": special_instructions
    })
}

// ── BOL response parser ────────────────────────────────────────

pub(crate) fn parse_bol_response(json: &Value) -> Result<LabelResult, CarrierProviderError> {
    // Accept both snake_case (Saia v2) and camelCase field names
    let pro_number = json["pro_number"]
        .as_str()
        .or_else(|| json["proNumber"].as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            CarrierProviderError::ApiError(
                "Saia BOL response missing 'pro_number'".to_string(),
            )
        })?;

    let bol_pdf_url = json["bol_pdf_url"]
        .as_str()
        .or_else(|| json["bolPdfUrl"].as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("")
        .to_string();

    Ok(LabelResult {
        tracking_number: pro_number.to_string(),
        label_format: "pdf_url".to_string(),
        label_data: bol_pdf_url,
        carrier_code: "saia".to_string(),
    })
}

// ── Track response parser ──────────────────────────────────────

pub(crate) fn parse_track_response(
    json: &Value,
    pro_number: &str,
) -> Result<TrackingResult, CarrierProviderError> {
    let status = json["status"].as_str().unwrap_or("UNKNOWN").to_string();

    let location = json["currentLocation"]
        .as_str()
        .or_else(|| json["current_location"].as_str())
        .map(|s| s.to_string());

    let estimated_delivery = json["estimatedDeliveryDate"]
        .as_str()
        .or_else(|| json["estimated_delivery_date"].as_str())
        .map(|s| s.to_string());

    let history = json["history"].as_array().cloned().unwrap_or_default();

    let events: Vec<TrackingEvent> = history
        .iter()
        .map(|h| {
            // Saia tracking history uses snake_case status_dttm per bead spec
            let timestamp = h["status_dttm"]
                .as_str()
                .or_else(|| h["statusDttm"].as_str())
                .unwrap_or("")
                .to_string();
            let description = h["status"].as_str().unwrap_or("Unknown").to_string();
            let evt_location = h["location"].as_str().map(|s| s.to_string());
            TrackingEvent {
                timestamp,
                description,
                location: evt_location,
            }
        })
        .collect();

    Ok(TrackingResult {
        tracking_number: pro_number.to_string(),
        carrier_code: "saia".to_string(),
        status,
        location,
        estimated_delivery,
        events,
    })
}

// ── CarrierProvider implementation ────────────────────────────

#[async_trait]
impl CarrierProvider for SaiaCarrierProvider {
    fn carrier_code(&self) -> &str {
        "saia"
    }

    async fn get_rates(
        &self,
        req: &Value,
        config: &Value,
    ) -> Result<Vec<RateQuote>, CarrierProviderError> {
        let api_key = get_api_key(config)?.to_string();
        let account_number = get_account_number(config)?.to_string();
        let base_url = get_base_url(config).to_string();
        let client = Client::new();
        let url = format!("{base_url}/rating/v2/quote");
        let body = build_rate_request(req, &account_number);
        let json = saia_post(&client, &api_key, &account_number, &url, &body).await?;
        parse_rate_response(&json)
    }

    async fn create_label(
        &self,
        req: &Value,
        config: &Value,
    ) -> Result<LabelResult, CarrierProviderError> {
        let api_key = get_api_key(config)?.to_string();
        let account_number = get_account_number(config)?.to_string();
        let base_url = get_base_url(config).to_string();
        let client = Client::new();
        let url = format!("{base_url}/shipments/v2/bol");
        let body = build_bol_request(req, &account_number);
        let json = saia_post(&client, &api_key, &account_number, &url, &body).await?;
        parse_bol_response(&json)
    }

    async fn track(
        &self,
        tracking_number: &str,
        config: &Value,
    ) -> Result<TrackingResult, CarrierProviderError> {
        let api_key = get_api_key(config)?.to_string();
        let account_number = get_account_number(config)?.to_string();
        let base_url = get_base_url(config).to_string();
        let client = Client::new();
        let url = format!("{base_url}/tracking/v2/{tracking_number}");
        let json = saia_get(&client, &api_key, &account_number, &url).await?;
        parse_track_response(&json, tracking_number)
    }
}

// ── Unit tests ─────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_api_key_returns_credentials_error() {
        let config = serde_json::json!({});
        assert!(matches!(
            get_api_key(&config),
            Err(CarrierProviderError::CredentialsError(_))
        ));
    }

    #[test]
    fn empty_api_key_returns_credentials_error() {
        let config = serde_json::json!({ "api_key": "" });
        assert!(matches!(
            get_api_key(&config),
            Err(CarrierProviderError::CredentialsError(_))
        ));
    }

    #[test]
    fn missing_account_number_returns_credentials_error() {
        let config = serde_json::json!({ "api_key": "key" });
        assert!(matches!(
            get_account_number(&config),
            Err(CarrierProviderError::CredentialsError(_))
        ));
    }

    #[test]
    fn empty_account_number_returns_credentials_error() {
        let config = serde_json::json!({ "api_key": "key", "account_number": "" });
        assert!(matches!(
            get_account_number(&config),
            Err(CarrierProviderError::CredentialsError(_))
        ));
    }

    #[test]
    fn get_base_url_defaults_to_saia_endpoint() {
        let config = serde_json::json!({});
        assert_eq!(get_base_url(&config), SAIA_DEFAULT_URL);
    }

    #[test]
    fn get_base_url_uses_provided_value() {
        let config = serde_json::json!({
            "base_url": "https://api.saiasecure.com/webservice-sandbox"
        });
        assert_eq!(
            get_base_url(&config),
            "https://api.saiasecure.com/webservice-sandbox"
        );
    }

    #[test]
    fn basic_auth_header_encodes_api_key_as_username_with_empty_password() {
        let header = basic_auth_header("mykey");
        assert!(header.starts_with("Basic "));
        // "mykey:" base64-encodes to "bXlrZXk6"
        assert_eq!(header, "Basic bXlrZXk6");
    }

    #[test]
    fn extract_saia_error_uses_message_field() {
        let json = serde_json::json!({ "message": "Invalid API key" });
        assert_eq!(extract_saia_error(&json, 401), "Invalid API key");
    }

    #[test]
    fn extract_saia_error_falls_back_to_error_field() {
        let json = serde_json::json!({ "error": "Not found" });
        assert_eq!(extract_saia_error(&json, 404), "Not found");
    }

    #[test]
    fn extract_saia_error_falls_back_to_error_message() {
        let json = serde_json::json!({ "errorMessage": "Account not authorized" });
        assert_eq!(extract_saia_error(&json, 422), "Account not authorized");
    }

    #[test]
    fn extract_saia_error_returns_fallback_on_empty() {
        let json = serde_json::json!({});
        assert_eq!(extract_saia_error(&json, 500), "unknown Saia API error");
    }

    #[test]
    fn build_rate_request_uses_req_fields_and_account_number() {
        let req = serde_json::json!({
            "origin_zip": "30301",
            "dest_zip": "94105",
            "weight_lbs": 750.0,
            "freight_class": "85",
        });
        let body = build_rate_request(&req, "SAI-001");
        assert_eq!(body["accountNumber"], "SAI-001");
        assert_eq!(body["origin"]["postalCode"], "30301");
        assert_eq!(body["destination"]["postalCode"], "94105");
        assert_eq!(body["commodities"][0]["weight"], 750.0);
        assert_eq!(body["commodities"][0]["freightClass"], "85");
    }

    #[test]
    fn build_rate_request_applies_defaults() {
        let body = build_rate_request(&serde_json::json!({}), "SAI-001");
        assert_eq!(body["origin"]["stateCode"], "GA");
        assert_eq!(body["destination"]["stateCode"], "CA");
        assert_eq!(body["commodities"][0]["freightClass"], "70");
    }

    #[test]
    fn build_bol_request_includes_account_number() {
        let req = serde_json::json!({
            "from_zip": "30301",
            "to_zip": "94105",
        });
        let body = build_bol_request(&req, "SAI-001");
        assert_eq!(body["accountNumber"], "SAI-001");
        assert_eq!(body["shipper"]["postalCode"], "30301");
        assert_eq!(body["consignee"]["postalCode"], "94105");
    }

    #[test]
    fn parse_rate_response_handles_services_array() {
        let json = serde_json::json!({
            "services": [
                {
                    "service_level": "Standard LTL",
                    "total_charge": 495.50,
                    "transit_days": 3
                },
                {
                    "service_level": "Guaranteed LTL",
                    "total_charge": 680.00,
                    "transit_days": 2
                }
            ]
        });
        let quotes = parse_rate_response(&json).expect("parse failed");
        assert_eq!(quotes.len(), 2);
        assert_eq!(quotes[0].service_level, "Standard LTL");
        assert_eq!(quotes[0].total_charge_minor, 49550);
        assert_eq!(quotes[0].carrier_code, "saia");
        assert_eq!(quotes[0].currency, "USD");
        assert_eq!(quotes[0].estimated_days, Some(3));
        assert_eq!(quotes[1].service_level, "Guaranteed LTL");
        assert_eq!(quotes[1].total_charge_minor, 68000);
        assert_eq!(quotes[1].estimated_days, Some(2));
    }

    #[test]
    fn parse_rate_response_handles_flat_single_rate() {
        let json = serde_json::json!({
            "service_level": "Standard LTL",
            "total_charge": 510.25,
            "transit_days": 4
        });
        let quotes = parse_rate_response(&json).expect("parse failed");
        assert_eq!(quotes.len(), 1);
        assert_eq!(quotes[0].service_level, "Standard LTL");
        assert_eq!(quotes[0].total_charge_minor, 51025);
        assert_eq!(quotes[0].carrier_code, "saia");
        assert_eq!(quotes[0].estimated_days, Some(4));
    }

    #[test]
    fn parse_rate_response_errors_on_empty_services() {
        let json = serde_json::json!({ "services": [] });
        assert!(parse_rate_response(&json).is_err());
    }

    #[test]
    fn parse_bol_response_extracts_snake_case_pro_number() {
        // Saia PRO numbers are 9-digit numeric — distinct from R&L/XPO/ODFL formats
        let json = serde_json::json!({
            "pro_number": "123456789",
            "bol_pdf_url": "https://api.saiasecure.com/bol/123456789.pdf"
        });
        let result = parse_bol_response(&json).expect("parse failed");
        assert_eq!(result.tracking_number, "123456789");
        assert_eq!(
            result.label_data,
            "https://api.saiasecure.com/bol/123456789.pdf"
        );
        assert_eq!(result.label_format, "pdf_url");
        assert_eq!(result.carrier_code, "saia");
    }

    #[test]
    fn parse_bol_response_accepts_camel_case_fields() {
        let json = serde_json::json!({
            "proNumber": "987654321",
            "bolPdfUrl": "https://api.saiasecure.com/bol/987654321.pdf"
        });
        let result = parse_bol_response(&json).expect("parse failed");
        assert_eq!(result.tracking_number, "987654321");
        assert_eq!(result.carrier_code, "saia");
    }

    #[test]
    fn parse_bol_response_errors_on_missing_pro_number() {
        let json = serde_json::json!({ "bol_pdf_url": "https://example.com/bol.pdf" });
        assert!(parse_bol_response(&json).is_err());
    }

    #[test]
    fn parse_track_response_extracts_status_and_events() {
        let json = serde_json::json!({
            "status": "IN_TRANSIT",
            "currentLocation": "Memphis, TN",
            "estimatedDeliveryDate": "2026-04-29",
            "history": [
                {
                    "status": "PICKED_UP",
                    "status_dttm": "2026-04-25T08:00:00",
                    "location": "Atlanta, GA"
                },
                {
                    "status": "IN_TRANSIT",
                    "status_dttm": "2026-04-26T02:00:00",
                    "location": "Memphis, TN"
                }
            ]
        });
        let result = parse_track_response(&json, "123456789").expect("parse failed");
        assert_eq!(result.tracking_number, "123456789");
        assert_eq!(result.carrier_code, "saia");
        assert_eq!(result.status, "IN_TRANSIT");
        assert_eq!(result.location, Some("Memphis, TN".to_string()));
        assert_eq!(result.estimated_delivery, Some("2026-04-29".to_string()));
        assert_eq!(result.events.len(), 2);
        assert_eq!(result.events[0].description, "PICKED_UP");
        assert_eq!(result.events[1].description, "IN_TRANSIT");
    }

    #[test]
    fn parse_track_response_handles_missing_history() {
        let json = serde_json::json!({ "status": "UNKNOWN" });
        let result = parse_track_response(&json, "000000000").expect("parse failed");
        assert_eq!(result.status, "UNKNOWN");
        assert!(result.events.is_empty());
        assert!(result.location.is_none());
    }
}
