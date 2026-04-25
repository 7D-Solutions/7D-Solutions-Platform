//! R&L Carriers LTL provider.
//!
//! Implements `CarrierProvider` for R&L Carriers via their REST API:
//! - Rate quotes:     POST /api/RateQuote
//! - Bill of Lading:  POST /api/BillOfLading
//! - Tracking:        GET  /api/Shipments/{pro_number}
//!
//! ## Config JSON
//! ```json
//! {
//!   "api_key":  "your-rl-api-key",
//!   "base_url": "https://api.rlcarriers.com"
//! }
//! ```
//! `base_url` is optional — defaults to the R&L production/sandbox endpoint.
//!
//! ## Auth
//! API-key only (paste-field credential); no OAuth. The key is forwarded as
//! `X-API-Key` on every request. Service JWTs are not used.

use async_trait::async_trait;
use reqwest::Client;
use serde_json::Value;

use super::{
    CarrierProvider, CarrierProviderError, LabelResult, MultiPackageLabelRequest,
    MultiPackageLabelResponse, RateQuote, TrackingEvent, TrackingResult,
};

const RL_DEFAULT_URL: &str = "https://api.rlcarriers.com";

pub struct RlCarrierProvider;

// ── Config helpers ─────────────────────────────────────────────

fn get_api_key(config: &Value) -> Result<&str, CarrierProviderError> {
    config["api_key"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            CarrierProviderError::CredentialsError(
                "R&L config missing required field 'api_key'".to_string(),
            )
        })
}

fn get_base_url(config: &Value) -> &str {
    config["base_url"]
        .as_str()
        .filter(|s| !s.is_empty())
        .unwrap_or(RL_DEFAULT_URL)
}

// ── HTTP helpers ───────────────────────────────────────────────

async fn rl_post(
    client: &Client,
    api_key: &str,
    url: &str,
    body: &Value,
) -> Result<Value, CarrierProviderError> {
    let resp = client
        .post(url)
        .header("X-API-Key", api_key)
        .header("Content-Type", "application/json")
        .json(body)
        .send()
        .await
        .map_err(|e| CarrierProviderError::ApiError(format!("R&L HTTP error: {e}")))?;

    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| CarrierProviderError::ApiError(format!("R&L response read error: {e}")))?;

    let json: Value = serde_json::from_str(&text).unwrap_or_else(|_| serde_json::json!({}));

    if !status.is_success() {
        let msg = extract_rl_error(&json, status.as_u16());
        return Err(CarrierProviderError::ApiError(format!(
            "R&L API error (HTTP {status}): {msg}"
        )));
    }

    Ok(json)
}

async fn rl_get(
    client: &Client,
    api_key: &str,
    url: &str,
) -> Result<Value, CarrierProviderError> {
    let resp = client
        .get(url)
        .header("X-API-Key", api_key)
        .send()
        .await
        .map_err(|e| CarrierProviderError::ApiError(format!("R&L HTTP error: {e}")))?;

    let status = resp.status();

    if status.as_u16() == 404 {
        return Err(CarrierProviderError::NotFound(
            "R&L: PRO number not found".to_string(),
        ));
    }

    let text = resp
        .text()
        .await
        .map_err(|e| CarrierProviderError::ApiError(format!("R&L response read error: {e}")))?;

    let json: Value = serde_json::from_str(&text).unwrap_or_else(|_| serde_json::json!({}));

    if !status.is_success() {
        let msg = extract_rl_error(&json, status.as_u16());
        return Err(CarrierProviderError::ApiError(format!(
            "R&L API error (HTTP {status}): {msg}"
        )));
    }

    Ok(json)
}

// ── Error extraction ───────────────────────────────────────────

pub(crate) fn extract_rl_error(json: &Value, _status: u16) -> String {
    if let Some(msg) = json["message"].as_str().filter(|s| !s.is_empty()) {
        return msg.to_string();
    }
    if let Some(err) = json["error"].as_str().filter(|s| !s.is_empty()) {
        return err.to_string();
    }
    "unknown R&L API error".to_string()
}

// ── Rate request builder ───────────────────────────────────────

pub(crate) fn build_rate_request(req: &Value) -> Value {
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
        "origin": {
            "zip": origin_zip,
            "city": origin_city,
            "state": origin_state
        },
        "destination": {
            "zip": dest_zip,
            "city": dest_city,
            "state": dest_state
        },
        "freight_items": [
            {
                "weight_lbs": weight_lbs,
                "freight_class": freight_class,
                "pieces": pieces,
                "description": description
            }
        ]
    })
}

// ── Rate response parser ───────────────────────────────────────

pub(crate) fn parse_rate_response(json: &Value) -> Result<Vec<RateQuote>, CarrierProviderError> {
    let services = json["services"].as_array().ok_or_else(|| {
        CarrierProviderError::ApiError("R&L rate response missing 'services' array".to_string())
    })?;

    if services.is_empty() {
        return Err(CarrierProviderError::ApiError(
            "R&L rate response returned no service options".to_string(),
        ));
    }

    let quotes = services
        .iter()
        .map(|s| {
            let service_level = s["service_level"].as_str().unwrap_or("LTL Standard").to_string();
            let total_charge = s["total_charge"].as_f64().unwrap_or(0.0);
            let total_charge_minor = (total_charge * 100.0).round() as i64;
            let estimated_days = s["transit_days"].as_i64().map(|d| d as i32);
            RateQuote {
                service_level,
                carrier_code: "rl".to_string(),
                total_charge_minor,
                currency: "USD".to_string(),
                estimated_days,
            }
        })
        .collect();

    Ok(quotes)
}

// ── BOL request builder ────────────────────────────────────────

pub(crate) fn build_bol_request(req: &Value) -> Value {
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
        "shipper": {
            "name": from_name,
            "address": from_address,
            "city": from_city,
            "state": from_state,
            "zip": from_zip
        },
        "consignee": {
            "name": to_name,
            "address": to_address,
            "city": to_city,
            "state": to_state,
            "zip": to_zip
        },
        "freight_items": [
            {
                "weight_lbs": weight_lbs,
                "freight_class": freight_class,
                "pieces": pieces,
                "description": description
            }
        ],
        "special_instructions": special_instructions
    })
}

// ── BOL response parser ────────────────────────────────────────

pub(crate) fn parse_bol_response(json: &Value) -> Result<LabelResult, CarrierProviderError> {
    let pro_number = json["pro_number"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            CarrierProviderError::ApiError("R&L BOL response missing 'pro_number'".to_string())
        })?;

    let bol_pdf_url = json["bol_pdf_url"]
        .as_str()
        .filter(|s| !s.is_empty())
        .unwrap_or("")
        .to_string();

    Ok(LabelResult {
        tracking_number: pro_number.to_string(),
        label_format: "pdf_url".to_string(),
        label_data: bol_pdf_url,
        carrier_code: "rl".to_string(),
    })
}

// ── Track response parser ──────────────────────────────────────

pub(crate) fn parse_track_response(
    json: &Value,
    pro_number: &str,
) -> Result<TrackingResult, CarrierProviderError> {
    let status = json["status"].as_str().unwrap_or("UNKNOWN").to_string();

    let location = json["location"].as_str().map(|s| s.to_string());

    let estimated_delivery = json["estimated_delivery_date"]
        .as_str()
        .map(|s| s.to_string());

    let history = json["history"].as_array().cloned().unwrap_or_default();

    let events: Vec<TrackingEvent> = history
        .iter()
        .map(|h| {
            let timestamp = h["dttm"].as_str().unwrap_or("").to_string();
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
        carrier_code: "rl".to_string(),
        status,
        location,
        estimated_delivery,
        events,
    })
}

// ── CarrierProvider implementation ────────────────────────────

#[async_trait]
impl CarrierProvider for RlCarrierProvider {
    fn carrier_code(&self) -> &str {
        "rl"
    }

    async fn get_rates(
        &self,
        req: &Value,
        config: &Value,
    ) -> Result<Vec<RateQuote>, CarrierProviderError> {
        let api_key = get_api_key(config)?.to_string();
        let base_url = get_base_url(config).to_string();
        let client = Client::new();
        let url = format!("{base_url}/api/RateQuote");
        let body = build_rate_request(req);
        let json = rl_post(&client, &api_key, &url, &body).await?;
        parse_rate_response(&json)
    }

    async fn create_label(
        &self,
        req: &Value,
        config: &Value,
    ) -> Result<LabelResult, CarrierProviderError> {
        let api_key = get_api_key(config)?.to_string();
        let base_url = get_base_url(config).to_string();
        let client = Client::new();
        let url = format!("{base_url}/api/BillOfLading");
        let body = build_bol_request(req);
        let json = rl_post(&client, &api_key, &url, &body).await?;
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
        let base_url = get_base_url(config).to_string();
        let client = Client::new();
        let url = format!("{base_url}/api/BillOfLading");
        let total_weight: f64 = req.packages.iter().map(|p| p.weight_lbs).sum();
        let bol_req = serde_json::json!({
            "shipper": {
                "name": req.origin["name"].as_str().unwrap_or("Shipper"),
                "address": req.origin["address"].as_str().unwrap_or(""),
                "city": req.origin["city"].as_str().unwrap_or(""),
                "state": req.origin["state"].as_str().unwrap_or(""),
                "zip": req.origin["zip"].as_str().unwrap_or("")
            },
            "consignee": {
                "name": req.destination["name"].as_str().unwrap_or("Consignee"),
                "address": req.destination["address"].as_str().unwrap_or(""),
                "city": req.destination["city"].as_str().unwrap_or(""),
                "state": req.destination["state"].as_str().unwrap_or(""),
                "zip": req.destination["zip"].as_str().unwrap_or("")
            },
            "freight_items": [{
                "weight_lbs": total_weight,
                "freight_class": "70",
                "pieces": req.packages.len(),
                "description": "Multi-piece LTL freight"
            }]
        });
        let json = rl_post(&client, &api_key, &url, &bol_req).await?;
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
        // R&L uses the same BOL endpoint for returns — caller provides swapped addresses.
        self.create_label(req, config).await
    }

    async fn track(
        &self,
        tracking_number: &str,
        config: &Value,
    ) -> Result<TrackingResult, CarrierProviderError> {
        let api_key = get_api_key(config)?.to_string();
        let base_url = get_base_url(config).to_string();
        let client = Client::new();
        let url = format!("{base_url}/api/Shipments/{tracking_number}");
        let json = rl_get(&client, &api_key, &url).await?;
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
    fn get_base_url_defaults_to_rl_endpoint() {
        let config = serde_json::json!({});
        assert_eq!(get_base_url(&config), RL_DEFAULT_URL);
    }

    #[test]
    fn get_base_url_uses_provided_value() {
        let config = serde_json::json!({ "base_url": "https://sandbox.rlcarriers.com" });
        assert_eq!(get_base_url(&config), "https://sandbox.rlcarriers.com");
    }

    #[test]
    fn extract_rl_error_uses_message_field() {
        let json = serde_json::json!({ "message": "Invalid API key" });
        assert_eq!(extract_rl_error(&json, 401), "Invalid API key");
    }

    #[test]
    fn extract_rl_error_falls_back_to_error_field() {
        let json = serde_json::json!({ "error": "Not found" });
        assert_eq!(extract_rl_error(&json, 404), "Not found");
    }

    #[test]
    fn extract_rl_error_returns_fallback_on_empty() {
        let json = serde_json::json!({});
        assert_eq!(extract_rl_error(&json, 500), "unknown R&L API error");
    }

    #[test]
    fn build_rate_request_uses_req_fields() {
        let req = serde_json::json!({
            "origin_zip": "30301",
            "dest_zip": "94105",
            "weight_lbs": 750.0,
            "freight_class": "85",
        });
        let body = build_rate_request(&req);
        assert_eq!(body["origin"]["zip"], "30301");
        assert_eq!(body["destination"]["zip"], "94105");
        assert_eq!(body["freight_items"][0]["weight_lbs"], 750.0);
        assert_eq!(body["freight_items"][0]["freight_class"], "85");
    }

    #[test]
    fn build_rate_request_applies_defaults() {
        let body = build_rate_request(&serde_json::json!({}));
        assert_eq!(body["origin"]["state"], "GA");
        assert_eq!(body["destination"]["state"], "CA");
        assert_eq!(body["freight_items"][0]["freight_class"], "70");
    }

    #[test]
    fn parse_rate_response_extracts_quotes() {
        let json = serde_json::json!({
            "services": [
                {
                    "service_level": "Standard LTL",
                    "total_charge": 450.50,
                    "transit_days": 3
                },
                {
                    "service_level": "Guaranteed LTL",
                    "total_charge": 620.00,
                    "transit_days": 2
                }
            ]
        });
        let quotes = parse_rate_response(&json).expect("parse failed");
        assert_eq!(quotes.len(), 2);
        assert_eq!(quotes[0].service_level, "Standard LTL");
        assert_eq!(quotes[0].total_charge_minor, 45050);
        assert_eq!(quotes[0].carrier_code, "rl");
        assert_eq!(quotes[0].currency, "USD");
        assert_eq!(quotes[0].estimated_days, Some(3));
        assert_eq!(quotes[1].service_level, "Guaranteed LTL");
        assert_eq!(quotes[1].total_charge_minor, 62000);
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
            "pro_number": "055-12345",
            "bol_pdf_url": "https://api.rlcarriers.com/bol/055-12345.pdf"
        });
        let result = parse_bol_response(&json).expect("parse failed");
        assert_eq!(result.tracking_number, "055-12345");
        assert_eq!(result.label_data, "https://api.rlcarriers.com/bol/055-12345.pdf");
        assert_eq!(result.label_format, "pdf_url");
        assert_eq!(result.carrier_code, "rl");
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
            "location": "Memphis, TN",
            "estimated_delivery_date": "2026-04-26",
            "history": [
                {
                    "status": "PICKED_UP",
                    "dttm": "2026-04-24T08:00:00",
                    "location": "Atlanta, GA"
                },
                {
                    "status": "IN_TRANSIT",
                    "dttm": "2026-04-25T02:30:00",
                    "location": "Memphis, TN"
                }
            ]
        });
        let result = parse_track_response(&json, "055-12345").expect("parse failed");
        assert_eq!(result.tracking_number, "055-12345");
        assert_eq!(result.carrier_code, "rl");
        assert_eq!(result.status, "IN_TRANSIT");
        assert_eq!(result.location, Some("Memphis, TN".to_string()));
        assert_eq!(result.estimated_delivery, Some("2026-04-26".to_string()));
        assert_eq!(result.events.len(), 2);
        assert_eq!(result.events[0].description, "PICKED_UP");
        assert_eq!(result.events[1].description, "IN_TRANSIT");
    }

    #[test]
    fn parse_track_response_handles_missing_history() {
        let json = serde_json::json!({ "status": "UNKNOWN" });
        let result = parse_track_response(&json, "000-00000").expect("parse failed");
        assert_eq!(result.status, "UNKNOWN");
        assert!(result.events.is_empty());
        assert!(result.location.is_none());
    }
}
