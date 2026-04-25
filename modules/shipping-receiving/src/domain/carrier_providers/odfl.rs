//! Old Dominion Freight Line (ODFL) LTL carrier provider.
//!
//! Implements `CarrierProvider` for ODFL via their REST API:
//! - Rate quotes:     POST /rating/1.0/quote
//! - Bill of Lading:  POST /bol/1.0
//! - Tracking:        GET  /tracking/1.0/{pro_number}
//! - Void:            POST /bol/1.0/{pro_number}/void
//!
//! ## Config JSON
//! ```json
//! {
//!   "api_key":        "your-odfl-api-key",
//!   "account_number": "your-odfl-account-number",
//!   "base_url":       "https://rest.odfl.com"
//! }
//! ```
//! `base_url` is optional — defaults to the ODFL production endpoint.
//! Use `https://rest.odfl.com/ODSWS-Sandbox` for sandbox testing.
//!
//! ## Auth
//! Both `api_key` and `account_number` are required. The API key goes in
//! `Authorization: Bearer {api_key}`; the account number goes in
//! `X-OD-Account-Number`. ODFL also requires the account number in the
//! BOL request body as the shipper reference — omitting it causes a 422.

use async_trait::async_trait;
use reqwest::Client;
use serde_json::Value;

use super::{
    CarrierProvider, CarrierProviderError, ChildLabel, LabelPdfResponse, LabelResult,
    MultiPackageLabelRequest, MultiPackageLabelResponse, RateQuote, TrackingEvent, TrackingResult,
};

const ODFL_DEFAULT_URL: &str = "https://rest.odfl.com";

pub struct OdflCarrierProvider;

// ── Config helpers ─────────────────────────────────────────────

fn get_api_key(config: &Value) -> Result<&str, CarrierProviderError> {
    config["api_key"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            CarrierProviderError::CredentialsError(
                "ODFL config missing required field 'api_key'".to_string(),
            )
        })
}

fn get_account_number(config: &Value) -> Result<&str, CarrierProviderError> {
    config["account_number"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            CarrierProviderError::CredentialsError(
                "ODFL config missing required field 'account_number'".to_string(),
            )
        })
}

fn get_base_url(config: &Value) -> &str {
    config["base_url"]
        .as_str()
        .filter(|s| !s.is_empty())
        .unwrap_or(ODFL_DEFAULT_URL)
}

// ── HTTP helpers ───────────────────────────────────────────────

async fn odfl_post(
    client: &Client,
    api_key: &str,
    account_number: &str,
    url: &str,
    body: &Value,
) -> Result<Value, CarrierProviderError> {
    let resp = client
        .post(url)
        .header("Authorization", format!("Bearer {api_key}"))
        .header("X-OD-Account-Number", account_number)
        .header("Content-Type", "application/json")
        .json(body)
        .send()
        .await
        .map_err(|e| CarrierProviderError::ApiError(format!("ODFL HTTP error: {e}")))?;

    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| CarrierProviderError::ApiError(format!("ODFL response read error: {e}")))?;

    let json: Value = serde_json::from_str(&text).unwrap_or_else(|_| serde_json::json!({}));

    if !status.is_success() {
        let msg = extract_odfl_error(&json, status.as_u16());
        return Err(CarrierProviderError::ApiError(format!(
            "ODFL API error (HTTP {status}): {msg}"
        )));
    }

    Ok(json)
}

async fn odfl_get(
    client: &Client,
    api_key: &str,
    account_number: &str,
    url: &str,
) -> Result<Value, CarrierProviderError> {
    let resp = client
        .get(url)
        .header("Authorization", format!("Bearer {api_key}"))
        .header("X-OD-Account-Number", account_number)
        .send()
        .await
        .map_err(|e| CarrierProviderError::ApiError(format!("ODFL HTTP error: {e}")))?;

    let status = resp.status();

    if status.as_u16() == 404 {
        return Err(CarrierProviderError::NotFound(
            "ODFL: PRO number not found".to_string(),
        ));
    }

    let text = resp
        .text()
        .await
        .map_err(|e| CarrierProviderError::ApiError(format!("ODFL response read error: {e}")))?;

    let json: Value = serde_json::from_str(&text).unwrap_or_else(|_| serde_json::json!({}));

    if !status.is_success() {
        let msg = extract_odfl_error(&json, status.as_u16());
        return Err(CarrierProviderError::ApiError(format!(
            "ODFL API error (HTTP {status}): {msg}"
        )));
    }

    Ok(json)
}

// ── Error extraction ───────────────────────────────────────────

pub(crate) fn extract_odfl_error(json: &Value, _status: u16) -> String {
    if let Some(msg) = json["message"].as_str().filter(|s| !s.is_empty()) {
        return msg.to_string();
    }
    if let Some(err) = json["error"].as_str().filter(|s| !s.is_empty()) {
        return err.to_string();
    }
    if let Some(desc) = json["errorMessage"].as_str().filter(|s| !s.is_empty()) {
        return desc.to_string();
    }
    "unknown ODFL API error".to_string()
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

pub(crate) fn parse_rate_response(json: &Value) -> Result<Vec<RateQuote>, CarrierProviderError> {
    let services = json["services"].as_array().ok_or_else(|| {
        CarrierProviderError::ApiError("ODFL rate response missing 'services' array".to_string())
    })?;

    if services.is_empty() {
        return Err(CarrierProviderError::ApiError(
            "ODFL rate response returned no service options".to_string(),
        ));
    }

    let quotes = services
        .iter()
        .map(|s| {
            let service_level = s["serviceLevel"]
                .as_str()
                .unwrap_or("LTL Standard")
                .to_string();
            let total_charge = s["totalCharge"].as_f64().unwrap_or(0.0);
            let total_charge_minor = (total_charge * 100.0).round() as i64;
            let estimated_days = s["transitDays"].as_i64().map(|d| d as i32);
            RateQuote {
                service_level,
                carrier_code: "odfl".to_string(),
                total_charge_minor,
                currency: "USD".to_string(),
                estimated_days,
            }
        })
        .collect();

    Ok(quotes)
}

// ── BOL request builder ────────────────────────────────────────

// ODFL requires account_number in the BOL body as shipperAccountNumber in
// addition to the X-OD-Account-Number header — omitting either causes a 422.
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
        "shipperAccountNumber": account_number,
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
    let pro_number = json["proNumber"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            CarrierProviderError::ApiError("ODFL BOL response missing 'proNumber'".to_string())
        })?;

    let bol_pdf_url = json["bolPdfUrl"]
        .as_str()
        .filter(|s| !s.is_empty())
        .unwrap_or("")
        .to_string();

    Ok(LabelResult {
        tracking_number: pro_number.to_string(),
        label_format: "pdf_url".to_string(),
        label_data: bol_pdf_url,
        carrier_code: "odfl".to_string(),
    })
}

// ── Track response parser ──────────────────────────────────────

pub(crate) fn parse_track_response(
    json: &Value,
    pro_number: &str,
) -> Result<TrackingResult, CarrierProviderError> {
    let status = json["status"].as_str().unwrap_or("UNKNOWN").to_string();

    let location = json["currentLocation"].as_str().map(|s| s.to_string());

    let estimated_delivery = json["estimatedDeliveryDate"]
        .as_str()
        .map(|s| s.to_string());

    let history = json["history"].as_array().cloned().unwrap_or_default();

    let events: Vec<TrackingEvent> = history
        .iter()
        .map(|h| {
            let timestamp = h["statusDttm"].as_str().unwrap_or("").to_string();
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
        carrier_code: "odfl".to_string(),
        status,
        location,
        estimated_delivery,
        events,
    })
}

// ── CarrierProvider implementation ────────────────────────────

#[async_trait]
impl CarrierProvider for OdflCarrierProvider {
    fn carrier_code(&self) -> &str {
        "odfl"
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
        let url = format!("{base_url}/rating/1.0/quote");
        let body = build_rate_request(req, &account_number);
        let json = odfl_post(&client, &api_key, &account_number, &url, &body).await?;
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
        let url = format!("{base_url}/bol/1.0");
        let body = build_bol_request(req, &account_number);
        let json = odfl_post(&client, &api_key, &account_number, &url, &body).await?;
        parse_bol_response(&json)
    }

    async fn create_multi_package_label(
        &self,
        req: &MultiPackageLabelRequest,
        config: &Value,
    ) -> Result<MultiPackageLabelResponse, CarrierProviderError> {
        if req.packages.is_empty() {
            return Err(CarrierProviderError::InvalidRequest(
                "packages must not be empty".to_string(),
            ));
        }
        // LTL: all handling units share one BOL (one pro_number). children is always empty.
        let api_key = get_api_key(config)?.to_string();
        let account_number = get_account_number(config)?.to_string();
        let base_url = get_base_url(config).to_string();
        let client = Client::new();
        let url = format!("{base_url}/bol/1.0");
        let total_weight: f64 = req.packages.iter().map(|p| p.weight_lbs).sum();
        let bol_req = serde_json::json!({
            "shipperAccountNumber": account_number,
            "shipper": {
                "name": req.origin["name"].as_str().unwrap_or("Shipper"),
                "address": req.origin["address"].as_str().unwrap_or(""),
                "city": req.origin["city"].as_str().unwrap_or(""),
                "stateCode": req.origin["state"].as_str().unwrap_or(""),
                "postalCode": req.origin["zip"].as_str().unwrap_or("")
            },
            "consignee": {
                "name": req.destination["name"].as_str().unwrap_or("Consignee"),
                "address": req.destination["address"].as_str().unwrap_or(""),
                "city": req.destination["city"].as_str().unwrap_or(""),
                "stateCode": req.destination["state"].as_str().unwrap_or(""),
                "postalCode": req.destination["zip"].as_str().unwrap_or("")
            },
            "commodities": [{
                "weight": total_weight,
                "freightClass": "70",
                "pieces": req.packages.len(),
                "description": "Multi-piece LTL freight"
            }]
        });
        let json = odfl_post(&client, &api_key, &account_number, &url, &bol_req).await?;
        let bol = parse_bol_response(&json)?;
        Ok(MultiPackageLabelResponse {
            master_tracking_number: bol.tracking_number,
            children: vec![],
        })
    }

    async fn create_return_label(
        &self,
        req: &Value,
        config: &Value,
    ) -> Result<LabelResult, CarrierProviderError> {
        // ODFL uses the same BOL endpoint for returns — caller provides swapped addresses.
        self.create_label(req, config).await
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
        let url = format!("{base_url}/tracking/1.0/{tracking_number}");
        let json = odfl_get(&client, &api_key, &account_number, &url).await?;
        parse_track_response(&json, tracking_number)
    }

    async fn fetch_label(
        &self,
        tracking_number: &str,
        config: &Value,
    ) -> Result<LabelPdfResponse, CarrierProviderError> {
        let api_key = get_api_key(config)?.to_string();
        let account_number = get_account_number(config)?.to_string();
        let base_url = get_base_url(config).to_string();
        let client = Client::new();

        let url = format!("{base_url}/bol/1.0/{tracking_number}");
        let json = odfl_get(&client, &api_key, &account_number, &url).await?;

        let pdf_url = json["bolPdfUrl"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                CarrierProviderError::NotFound(format!(
                    "ODFL: no BOL PDF URL for PRO {tracking_number}"
                ))
            })?;

        let pdf_resp = client
            .get(pdf_url)
            .send()
            .await
            .map_err(|e| CarrierProviderError::ApiError(format!("ODFL PDF fetch error: {e}")))?;

        if !pdf_resp.status().is_success() {
            return Err(CarrierProviderError::ApiError(format!(
                "ODFL PDF fetch HTTP {}: {}",
                pdf_resp.status(),
                pdf_url
            )));
        }

        let pdf_bytes = pdf_resp.bytes().await.map_err(|e| {
            CarrierProviderError::ApiError(format!("ODFL PDF read error: {e}"))
        })?;

        Ok(LabelPdfResponse {
            pdf_bytes: pdf_bytes.to_vec(),
            content_type: "application/pdf".to_string(),
            carrier_reference: tracking_number.to_string(),
        })
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
    fn get_base_url_defaults_to_odfl_endpoint() {
        let config = serde_json::json!({});
        assert_eq!(get_base_url(&config), ODFL_DEFAULT_URL);
    }

    #[test]
    fn get_base_url_uses_provided_value() {
        let config =
            serde_json::json!({ "base_url": "https://rest.odfl.com/ODSWS-Sandbox" });
        assert_eq!(get_base_url(&config), "https://rest.odfl.com/ODSWS-Sandbox");
    }

    #[test]
    fn extract_odfl_error_uses_message_field() {
        let json = serde_json::json!({ "message": "Invalid API key" });
        assert_eq!(extract_odfl_error(&json, 401), "Invalid API key");
    }

    #[test]
    fn extract_odfl_error_falls_back_to_error_field() {
        let json = serde_json::json!({ "error": "Not found" });
        assert_eq!(extract_odfl_error(&json, 404), "Not found");
    }

    #[test]
    fn extract_odfl_error_falls_back_to_error_message() {
        let json = serde_json::json!({ "errorMessage": "Account not authorized" });
        assert_eq!(extract_odfl_error(&json, 422), "Account not authorized");
    }

    #[test]
    fn extract_odfl_error_returns_fallback_on_empty() {
        let json = serde_json::json!({});
        assert_eq!(extract_odfl_error(&json, 500), "unknown ODFL API error");
    }

    #[test]
    fn build_rate_request_uses_req_fields_and_account_number() {
        let req = serde_json::json!({
            "origin_zip": "30301",
            "dest_zip": "94105",
            "weight_lbs": 750.0,
            "freight_class": "85",
        });
        let body = build_rate_request(&req, "ACC-001");
        assert_eq!(body["accountNumber"], "ACC-001");
        assert_eq!(body["origin"]["postalCode"], "30301");
        assert_eq!(body["destination"]["postalCode"], "94105");
        assert_eq!(body["commodities"][0]["weight"], 750.0);
        assert_eq!(body["commodities"][0]["freightClass"], "85");
    }

    #[test]
    fn build_rate_request_applies_defaults() {
        let body = build_rate_request(&serde_json::json!({}), "ACC-001");
        assert_eq!(body["origin"]["stateCode"], "GA");
        assert_eq!(body["destination"]["stateCode"], "CA");
        assert_eq!(body["commodities"][0]["freightClass"], "70");
    }

    #[test]
    fn build_bol_request_includes_account_number_in_body() {
        let req = serde_json::json!({
            "from_zip": "30301",
            "to_zip": "94105",
        });
        let body = build_bol_request(&req, "ACC-001");
        assert_eq!(body["shipperAccountNumber"], "ACC-001");
        assert_eq!(body["shipper"]["postalCode"], "30301");
        assert_eq!(body["consignee"]["postalCode"], "94105");
    }

    #[test]
    fn parse_rate_response_extracts_quotes() {
        let json = serde_json::json!({
            "services": [
                {
                    "serviceLevel": "Standard LTL",
                    "totalCharge": 480.75,
                    "transitDays": 3
                },
                {
                    "serviceLevel": "Guaranteed LTL",
                    "totalCharge": 650.00,
                    "transitDays": 2
                }
            ]
        });
        let quotes = parse_rate_response(&json).expect("parse failed");
        assert_eq!(quotes.len(), 2);
        assert_eq!(quotes[0].service_level, "Standard LTL");
        assert_eq!(quotes[0].total_charge_minor, 48075);
        assert_eq!(quotes[0].carrier_code, "odfl");
        assert_eq!(quotes[0].currency, "USD");
        assert_eq!(quotes[0].estimated_days, Some(3));
        assert_eq!(quotes[1].service_level, "Guaranteed LTL");
        assert_eq!(quotes[1].total_charge_minor, 65000);
        assert_eq!(quotes[1].estimated_days, Some(2));
    }

    #[test]
    fn parse_rate_response_errors_on_missing_services() {
        let json = serde_json::json!({});
        assert!(parse_rate_response(&json).is_err());
    }

    #[test]
    fn parse_rate_response_errors_on_empty_services() {
        let json = serde_json::json!({ "services": [] });
        assert!(parse_rate_response(&json).is_err());
    }

    #[test]
    fn parse_bol_response_extracts_pro_number() {
        let json = serde_json::json!({
            "proNumber": "12345678901",
            "bolPdfUrl": "https://rest.odfl.com/bol/12345678901.pdf",
            "pickupNumber": "PU-987654"
        });
        let result = parse_bol_response(&json).expect("parse failed");
        assert_eq!(result.tracking_number, "12345678901");
        assert_eq!(result.label_data, "https://rest.odfl.com/bol/12345678901.pdf");
        assert_eq!(result.label_format, "pdf_url");
        assert_eq!(result.carrier_code, "odfl");
    }

    #[test]
    fn parse_bol_response_errors_on_missing_pro_number() {
        let json = serde_json::json!({ "bolPdfUrl": "https://example.com/bol.pdf" });
        assert!(parse_bol_response(&json).is_err());
    }

    #[test]
    fn parse_track_response_extracts_status_and_events() {
        let json = serde_json::json!({
            "status": "IN_TRANSIT",
            "currentLocation": "Charlotte, NC",
            "estimatedDeliveryDate": "2026-04-27",
            "history": [
                {
                    "status": "PICKED_UP",
                    "statusDttm": "2026-04-25T09:00:00",
                    "location": "Atlanta, GA"
                },
                {
                    "status": "IN_TRANSIT",
                    "statusDttm": "2026-04-26T03:00:00",
                    "location": "Charlotte, NC"
                }
            ]
        });
        let result = parse_track_response(&json, "12345678901").expect("parse failed");
        assert_eq!(result.tracking_number, "12345678901");
        assert_eq!(result.carrier_code, "odfl");
        assert_eq!(result.status, "IN_TRANSIT");
        assert_eq!(result.location, Some("Charlotte, NC".to_string()));
        assert_eq!(result.estimated_delivery, Some("2026-04-27".to_string()));
        assert_eq!(result.events.len(), 2);
        assert_eq!(result.events[0].description, "PICKED_UP");
        assert_eq!(result.events[1].description, "IN_TRANSIT");
    }

    #[test]
    fn parse_track_response_handles_missing_history() {
        let json = serde_json::json!({ "status": "UNKNOWN" });
        let result = parse_track_response(&json, "00000000000").expect("parse failed");
        assert_eq!(result.status, "UNKNOWN");
        assert!(result.events.is_empty());
        assert!(result.location.is_none());
    }
}
