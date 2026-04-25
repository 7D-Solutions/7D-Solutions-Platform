//! FedEx REST API carrier provider.
//!
//! Implements `CarrierProvider` for FedEx via the REST API v1:
//! - OAuth2:        POST /oauth/token (client credentials, cached per client_id)
//! - Rate quotes:   POST /rate/v1/rates/quotes
//! - Label creation: POST /ship/v1/shipments
//! - Tracking:      POST /track/v1/trackingnumbers
//!
//! ## Config JSON
//! ```json
//! {
//!   "client_id":      "your_client_id",
//!   "client_secret":  "your_client_secret",
//!   "account_number": "your_account_number",
//!   "base_url":       "https://apis-sandbox.fedex.com"
//! }
//! ```
//! `base_url` is optional — defaults to the FedEx production endpoint.
//!
//! ## Invariant
//! Provider struct is zero-state. `config` is read on every call. OAuth2
//! tokens are cached in a module-level store keyed by `client_id` and
//! refreshed 60 seconds before actual expiry.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use base64::Engine as _;
use reqwest::Client;
use serde_json::Value;

use super::{
    CarrierProvider, CarrierProviderError, ChildLabel, LabelPdfResponse, LabelResult,
    MultiPackageLabelRequest, MultiPackageLabelResponse, RateQuote, TrackingEvent, TrackingResult,
};

const FEDEX_PRODUCTION_URL: &str = "https://apis.fedex.com";

// ── Token cache ───────────────────────────────────────────────
//
// Provider is zero-state, so we keep the cache at module level.
// Key = client_id; value = (access_token, expiry instant).

struct CachedToken {
    access_token: String,
    expires_at: Instant,
}

static TOKEN_CACHE: OnceLock<Mutex<HashMap<String, CachedToken>>> = OnceLock::new();

fn token_cache() -> &'static Mutex<HashMap<String, CachedToken>> {
    TOKEN_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn get_cached_token(client_id: &str) -> Option<String> {
    let cache = token_cache().lock().expect("token cache mutex poisoned");
    cache.get(client_id).and_then(|entry| {
        if Instant::now() < entry.expires_at {
            Some(entry.access_token.clone())
        } else {
            None
        }
    })
}

fn store_cached_token(client_id: &str, access_token: String, expires_in_secs: u64) {
    let mut cache = token_cache().lock().expect("token cache mutex poisoned");
    // Subtract 60 s buffer so we refresh before the token actually expires.
    let ttl = Duration::from_secs(expires_in_secs.saturating_sub(60));
    cache.insert(
        client_id.to_string(),
        CachedToken {
            access_token,
            expires_at: Instant::now() + ttl,
        },
    );
}

pub struct FedexCarrierProvider;

// ── Credential helpers ────────────────────────────────────────

fn get_client_id(config: &Value) -> Result<&str, CarrierProviderError> {
    config["client_id"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            CarrierProviderError::CredentialsError(
                "FedEx config missing required field 'client_id'".to_string(),
            )
        })
}

fn get_client_secret(config: &Value) -> Result<&str, CarrierProviderError> {
    config["client_secret"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            CarrierProviderError::CredentialsError(
                "FedEx config missing required field 'client_secret'".to_string(),
            )
        })
}

fn get_account_number(config: &Value) -> Result<&str, CarrierProviderError> {
    config["account_number"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            CarrierProviderError::CredentialsError(
                "FedEx config missing required field 'account_number'".to_string(),
            )
        })
}

fn get_base_url(config: &Value) -> &str {
    config["base_url"]
        .as_str()
        .filter(|s| !s.is_empty())
        .unwrap_or(FEDEX_PRODUCTION_URL)
}

// ── OAuth2 token acquisition ──────────────────────────────────

async fn acquire_token(
    base_url: &str,
    client_id: &str,
    client_secret: &str,
) -> Result<String, CarrierProviderError> {
    if let Some(token) = get_cached_token(client_id) {
        return Ok(token);
    }

    let client = Client::new();
    let oauth_url = format!("{}/oauth/token", base_url.trim_end_matches('/'));

    let resp = client
        .post(&oauth_url)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(format!(
            "grant_type=client_credentials&client_id={}&client_secret={}",
            client_id, client_secret
        ))
        .send()
        .await
        .map_err(|e| {
            CarrierProviderError::CredentialsError(format!("FedEx OAuth HTTP error: {e}"))
        })?;

    let status = resp.status();
    let body: Value = resp.json().await.map_err(|e| {
        CarrierProviderError::CredentialsError(format!("FedEx OAuth response parse error: {e}"))
    })?;

    if !status.is_success() {
        let msg = body["errors"]
            .as_array()
            .and_then(|a| a.first())
            .and_then(|e| e["message"].as_str())
            .or_else(|| body["error_description"].as_str())
            .unwrap_or("unknown OAuth error");
        return Err(CarrierProviderError::CredentialsError(format!(
            "FedEx OAuth {status}: {msg}"
        )));
    }

    let access_token = body["access_token"]
        .as_str()
        .ok_or_else(|| {
            CarrierProviderError::CredentialsError(
                "FedEx OAuth response missing access_token".to_string(),
            )
        })?
        .to_string();

    let expires_in = body["expires_in"].as_u64().unwrap_or(3600);
    store_cached_token(client_id, access_token.clone(), expires_in);

    Ok(access_token)
}

// ── HTTP helpers ──────────────────────────────────────────────

async fn fedex_post(
    base_url: &str,
    path: &str,
    token: &str,
    body: Value,
) -> Result<Value, CarrierProviderError> {
    let client = Client::new();
    let url = format!("{}{}", base_url.trim_end_matches('/'), path);

    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {token}"))
        .header("X-locale", "en_US")
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| CarrierProviderError::ApiError(format!("FedEx HTTP error: {e}")))?;

    let status = resp.status();
    let response_body: Value = resp
        .json()
        .await
        .map_err(|e| CarrierProviderError::ApiError(format!("FedEx response parse error: {e}")))?;

    if !status.is_success() {
        let msg = extract_fedex_error(&response_body);
        return Err(CarrierProviderError::ApiError(format!(
            "FedEx {status}: {msg}"
        )));
    }

    Ok(response_body)
}

/// Extract a human-readable error message from a FedEx API error response.
fn extract_fedex_error(body: &Value) -> String {
    body["errors"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(|e| e["message"].as_str())
        .or_else(|| {
            body["output"]["alerts"]
                .as_array()
                .and_then(|a| a.first())
                .and_then(|a| a["message"].as_str())
        })
        .unwrap_or("unknown FedEx API error")
        .to_string()
}

// ── Rate request builder ──────────────────────────────────────

fn build_rate_request(account_number: &str, req: &Value) -> Value {
    let from_zip = req["origin_zip"].as_str().unwrap_or("10001");
    let to_zip = req["dest_zip"].as_str().unwrap_or("90210");
    let weight = req["weight_lbs"].as_f64().unwrap_or(1.0);
    let length = req["length_in"].as_f64().unwrap_or(12.0);
    let width = req["width_in"].as_f64().unwrap_or(12.0);
    let height = req["height_in"].as_f64().unwrap_or(12.0);

    serde_json::json!({
        "accountNumber": { "value": account_number },
        "rateRequestType": ["ACCOUNT", "LIST"],
        "requestedShipment": {
            "shipper": {
                "address": { "postalCode": from_zip, "countryCode": "US" }
            },
            "recipient": {
                "address": { "postalCode": to_zip, "countryCode": "US" }
            },
            "pickupType": "USE_SCHEDULED_PICKUP",
            "requestedPackageLineItems": [{
                "weight": { "units": "LB", "value": weight },
                "dimensions": {
                    "length": length,
                    "width":  width,
                    "height": height,
                    "units":  "IN"
                }
            }]
        }
    })
}

// ── Rate response parser ──────────────────────────────────────

/// Convert FedEx dollar amounts (f64) to integer minor units (cents).
fn dollars_to_minor(v: f64) -> i64 {
    (v * 100.0).round() as i64
}

/// Convert FedEx `transitTime` enum strings to integer days.
///
/// FedEx Ground and Express services use different string values; this
/// normalises them all to a day count.  Unknown strings return `None`.
fn parse_transit_time(s: &str) -> Option<i32> {
    match s {
        "ONE_DAY" | "OVERNIGHT" => Some(1),
        "TWO_DAYS" => Some(2),
        "THREE_DAYS" => Some(3),
        "FOUR_DAYS" => Some(4),
        "FIVE_DAYS" => Some(5),
        "SIX_DAYS" => Some(6),
        "SEVEN_DAYS" => Some(7),
        _ => s.parse::<i32>().ok(),
    }
}

/// Extract estimated transit days from a FedEx service detail element.
///
/// Transit time may appear in several locations depending on service type:
/// - `ratedPackages[0].packageRateDetail.transitTime`   (most services)
/// - `commit.transitTime`                               (Express services)
/// - `commit.daysInTransit` (integer string, e.g. "1")  (some Express services)
fn extract_transit_days(service_detail: &Value, shipment_detail: &Value) -> Option<i32> {
    let transit_str = shipment_detail["ratedPackages"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(|p| p["packageRateDetail"]["transitTime"].as_str())
        .or_else(|| service_detail["commit"]["transitTime"].as_str())
        .or_else(|| service_detail["commit"]["daysInTransit"].as_str());

    transit_str.and_then(parse_transit_time)
}

/// Parse a FedEx Rate API response into `Vec<RateQuote>`.
///
/// FedEx returns one element per service type. `ratedShipmentDetails` is an
/// array with LIST and ACCOUNT rate entries. We prefer ACCOUNT; fall back to
/// the first element. Both `totalNetCharge` (top-level) and the nested
/// `shipmentRateDetail.totalNetCharge` (seen on some service types) are
/// handled.
fn parse_rate_response(body: &Value) -> Result<Vec<RateQuote>, CarrierProviderError> {
    let details = body["output"]["rateReplyDetails"]
        .as_array()
        .ok_or_else(|| {
            CarrierProviderError::ApiError(
                "FedEx Rate response missing output.rateReplyDetails".to_string(),
            )
        })?;

    if details.is_empty() {
        return Err(CarrierProviderError::ApiError(
            "FedEx Rate API returned no rate details".to_string(),
        ));
    }

    let mut quotes: Vec<RateQuote> = Vec::new();

    for detail in details {
        let service_name = detail["serviceName"]
            .as_str()
            .or_else(|| detail["serviceType"].as_str())
            .unwrap_or("Unknown Service")
            .to_string();

        let shipment_details = match detail["ratedShipmentDetails"].as_array() {
            Some(arr) if !arr.is_empty() => arr,
            _ => continue,
        };

        // Prefer ACCOUNT rate type; otherwise use the first available entry.
        let chosen = shipment_details
            .iter()
            .find(|d| d["rateType"].as_str() == Some("ACCOUNT"))
            .unwrap_or(&shipment_details[0]);

        // FedEx Ground uses top-level totalNetCharge; some Express services
        // nest it under shipmentRateDetail — check both.
        let charge_dollars = chosen["totalNetCharge"]
            .as_f64()
            .or_else(|| chosen["shipmentRateDetail"]["totalNetCharge"].as_f64())
            .unwrap_or(0.0);

        let currency = chosen["currency"]
            .as_str()
            .or_else(|| chosen["shipmentRateDetail"]["currency"].as_str())
            .unwrap_or("USD")
            .to_string();

        let estimated_days = extract_transit_days(detail, chosen);

        quotes.push(RateQuote {
            service_level: service_name,
            carrier_code: "fedex".to_string(),
            total_charge_minor: dollars_to_minor(charge_dollars),
            currency,
            estimated_days,
        });
    }

    if quotes.is_empty() {
        return Err(CarrierProviderError::ApiError(
            "FedEx Rate API returned no parseable quotes".to_string(),
        ));
    }

    Ok(quotes)
}

// ── Ship request builder ──────────────────────────────────────

fn build_ship_request(account_number: &str, req: &Value) -> Value {
    let from_name = req["from_name"].as_str().unwrap_or("Sender");
    let from_address = req["from_address"].as_str().unwrap_or("123 Main St");
    let from_city = req["from_city"].as_str().unwrap_or("New York");
    let from_state = req["from_state"].as_str().unwrap_or("NY");
    let from_zip = req["from_zip"].as_str().unwrap_or("10001");
    let to_name = req["to_name"].as_str().unwrap_or("Recipient");
    let to_address = req["to_address"].as_str().unwrap_or("456 Sunset Blvd");
    let to_city = req["to_city"].as_str().unwrap_or("Beverly Hills");
    let to_state = req["to_state"].as_str().unwrap_or("CA");
    let to_zip = req["to_zip"].as_str().unwrap_or("90210");
    let weight = req["weight_lbs"].as_f64().unwrap_or(1.0);
    let length = req["length_in"].as_f64().unwrap_or(12.0);
    let width = req["width_in"].as_f64().unwrap_or(12.0);
    let height = req["height_in"].as_f64().unwrap_or(12.0);
    let service_type = req["service_type"].as_str().unwrap_or("FEDEX_GROUND");

    serde_json::json!({
        "labelResponseOptions": "LABEL",
        "requestedShipment": {
            "shipper": {
                "contact": { "personName": from_name, "companyName": "" },
                "address": {
                    "streetLines": [from_address],
                    "city": from_city,
                    "stateOrProvinceCode": from_state,
                    "postalCode": from_zip,
                    "countryCode": "US"
                }
            },
            "recipients": [{
                "contact": { "personName": to_name },
                "address": {
                    "streetLines": [to_address],
                    "city": to_city,
                    "stateOrProvinceCode": to_state,
                    "postalCode": to_zip,
                    "countryCode": "US"
                }
            }],
            "serviceType": service_type,
            "packagingType": "YOUR_PACKAGING",
            "pickupType": "USE_SCHEDULED_PICKUP",
            "shippingChargesPayment": {
                "paymentType": "SENDER",
                "payor": {
                    "responsibleParty": {
                        "accountNumber": { "value": account_number }
                    }
                }
            },
            "labelSpecification": {
                "labelFormatType": "COMMON2D",
                "imageType": "PDF",
                "labelStockType": "PAPER_85X11_TOP_HALF_LABEL"
            },
            "requestedPackageLineItems": [{
                "weight": { "units": "LB", "value": weight },
                "dimensions": {
                    "length": length,
                    "width":  width,
                    "height": height,
                    "units":  "IN"
                }
            }]
        },
        "accountNumber": { "value": account_number }
    })
}

// ── Multi-package ship request builder ────────────────────────

fn build_multi_ship_request(account_number: &str, req: &MultiPackageLabelRequest) -> Value {
    let from_name = req.origin["name"].as_str().unwrap_or("Sender");
    let from_address = req.origin["address"].as_str().unwrap_or("123 Main St");
    let from_city = req.origin["city"].as_str().unwrap_or("New York");
    let from_state = req.origin["state"].as_str().unwrap_or("NY");
    let from_zip = req.origin["zip"].as_str().unwrap_or("10001");
    let to_name = req.destination["name"].as_str().unwrap_or("Recipient");
    let to_address = req.destination["address"].as_str().unwrap_or("456 Sunset Blvd");
    let to_city = req.destination["city"].as_str().unwrap_or("Los Angeles");
    let to_state = req.destination["state"].as_str().unwrap_or("CA");
    let to_zip = req.destination["zip"].as_str().unwrap_or("90210");
    let service_type = req
        .service_level
        .as_deref()
        .unwrap_or("FEDEX_GROUND");

    let pkg_count = req.packages.len() as u64;
    let total_weight: f64 = req.packages.iter().map(|p| p.weight_lbs).sum();

    let package_line_items: Vec<Value> = req
        .packages
        .iter()
        .enumerate()
        .map(|(i, pkg)| {
            serde_json::json!({
                "sequenceNumber": i + 1,
                "weight": { "units": "LB", "value": pkg.weight_lbs },
                "dimensions": {
                    "length": pkg.length_in,
                    "width":  pkg.width_in,
                    "height": pkg.height_in,
                    "units":  "IN"
                }
            })
        })
        .collect();

    serde_json::json!({
        "labelResponseOptions": "LABEL",
        "requestedShipment": {
            "shipper": {
                "contact": { "personName": from_name },
                "address": {
                    "streetLines": [from_address], "city": from_city,
                    "stateOrProvinceCode": from_state, "postalCode": from_zip, "countryCode": "US"
                }
            },
            "recipients": [{
                "contact": { "personName": to_name },
                "address": {
                    "streetLines": [to_address], "city": to_city,
                    "stateOrProvinceCode": to_state, "postalCode": to_zip, "countryCode": "US"
                }
            }],
            "serviceType": service_type,
            "packagingType": "YOUR_PACKAGING",
            "pickupType": "USE_SCHEDULED_PICKUP",
            "totalWeight": total_weight,
            "packageCount": pkg_count,
            "shippingChargesPayment": {
                "paymentType": "SENDER",
                "payor": { "responsibleParty": { "accountNumber": { "value": account_number } } }
            },
            "labelSpecification": {
                "labelFormatType": "COMMON2D", "imageType": "PDF",
                "labelStockType": "PAPER_85X11_TOP_HALF_LABEL"
            },
            "requestedPackageLineItems": package_line_items
        },
        "accountNumber": { "value": account_number }
    })
}

fn parse_multi_ship_response(body: &Value) -> Result<MultiPackageLabelResponse, CarrierProviderError> {
    let shipments = body["output"]["transactionShipments"]
        .as_array()
        .ok_or_else(|| {
            CarrierProviderError::ApiError(
                "FedEx multi-ship response missing output.transactionShipments".to_string(),
            )
        })?;

    let shipment = shipments.first().ok_or_else(|| {
        CarrierProviderError::ApiError(
            "FedEx multi-ship response has empty transactionShipments".to_string(),
        )
    })?;

    let master_tn = shipment["masterTrackingNumber"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            CarrierProviderError::ApiError(
                "FedEx multi-ship response missing masterTrackingNumber".to_string(),
            )
        })?;

    let piece_responses = shipment["pieceResponses"].as_array().cloned().unwrap_or_default();

    let children: Vec<ChildLabel> = piece_responses
        .iter()
        .enumerate()
        .map(|(i, piece)| {
            let tn = piece["trackingNumber"].as_str().unwrap_or("").to_string();
            let label = piece["packageDocuments"]
                .as_array()
                .and_then(|docs| {
                    docs.iter()
                        .find(|d| d["contentType"].as_str() == Some("LABEL"))
                        .and_then(|d| d["encodedLabel"].as_str())
                })
                .unwrap_or("")
                .to_string();
            ChildLabel {
                tracking_number: tn,
                label_url: label,
                package_index: i,
            }
        })
        .collect();

    Ok(MultiPackageLabelResponse {
        master_tracking_number: master_tn.to_string(),
        children,
    })
}

// ── Ship response parser ──────────────────────────────────────

fn parse_ship_response(body: &Value) -> Result<LabelResult, CarrierProviderError> {
    let shipments = body["output"]["transactionShipments"]
        .as_array()
        .ok_or_else(|| {
            CarrierProviderError::ApiError(
                "FedEx Ship response missing output.transactionShipments".to_string(),
            )
        })?;

    let shipment = shipments.first().ok_or_else(|| {
        CarrierProviderError::ApiError(
            "FedEx Ship response has empty transactionShipments".to_string(),
        )
    })?;

    let master_tracking = shipment["masterTrackingNumber"]
        .as_str()
        .unwrap_or("")
        .to_string();

    let piece_responses = shipment["pieceResponses"].as_array().ok_or_else(|| {
        CarrierProviderError::ApiError("FedEx Ship response missing pieceResponses".to_string())
    })?;

    let piece = piece_responses.first().ok_or_else(|| {
        CarrierProviderError::ApiError("FedEx Ship response has empty pieceResponses".to_string())
    })?;

    // Piece tracking number is authoritative; fall back to master for
    // single-piece shipments where the two values are identical.
    let tracking_number = piece["trackingNumber"]
        .as_str()
        .filter(|s| !s.is_empty())
        .unwrap_or(master_tracking.as_str())
        .to_string();

    if tracking_number.is_empty() {
        return Err(CarrierProviderError::ApiError(
            "FedEx Ship response contains no tracking number".to_string(),
        ));
    }

    // encodedLabel is present when labelResponseOptions = "LABEL".
    let label_data = piece["packageDocuments"]
        .as_array()
        .and_then(|docs| {
            docs.iter()
                .find(|d| d["contentType"].as_str() == Some("LABEL"))
                .and_then(|d| d["encodedLabel"].as_str())
        })
        .unwrap_or("")
        .to_string();

    Ok(LabelResult {
        tracking_number,
        label_format: "pdf".to_string(),
        label_data,
        carrier_code: "fedex".to_string(),
    })
}

// ── Track request builder ─────────────────────────────────────

fn build_track_request(tracking_number: &str) -> Value {
    serde_json::json!({
        "trackingInfo": [{
            "trackingNumberInfo": {
                "trackingNumber": tracking_number
            }
        }],
        "includeDetailedScans": true
    })
}

// ── Track response parser ─────────────────────────────────────

fn parse_track_response(
    body: &Value,
    tracking_number: &str,
) -> Result<TrackingResult, CarrierProviderError> {
    let complete_results = body["output"]["completeTrackResults"]
        .as_array()
        .ok_or_else(|| {
            CarrierProviderError::ApiError(
                "FedEx Track response missing output.completeTrackResults".to_string(),
            )
        })?;

    let result = complete_results.first().ok_or_else(|| {
        CarrierProviderError::ApiError(
            "FedEx Track response has empty completeTrackResults".to_string(),
        )
    })?;

    let track_results = result["trackResults"].as_array().ok_or_else(|| {
        CarrierProviderError::ApiError("FedEx Track response missing trackResults".to_string())
    })?;

    let track = track_results.first().ok_or_else(|| {
        CarrierProviderError::ApiError("FedEx Track response has empty trackResults".to_string())
    })?;

    // FedEx returns a per-result error for unknown tracking numbers.
    if let Some(err) = track.get("error") {
        let msg = err["message"].as_str().unwrap_or("FedEx tracking error");
        return Err(CarrierProviderError::ApiError(format!(
            "FedEx Track: {msg}"
        )));
    }

    let status = track["latestStatusDetail"]["description"]
        .as_str()
        .or_else(|| track["latestStatusDetail"]["statusByLocale"].as_str())
        .unwrap_or("UNKNOWN")
        .to_string();

    let scan_events = track["scanEvents"].as_array();

    // Current location = city/state of the most recent scan event.
    let location = scan_events.and_then(|evs| evs.first()).and_then(|ev| {
        let city = ev["scanLocation"]["city"].as_str().unwrap_or("");
        let state = ev["scanLocation"]["stateOrProvinceCode"]
            .as_str()
            .unwrap_or("");
        if city.is_empty() && state.is_empty() {
            None
        } else {
            Some(
                format!("{city}, {state}")
                    .trim_matches(',')
                    .trim()
                    .to_string(),
            )
        }
    });

    // Estimated delivery from the ESTIMATED_DELIVERY date entry.
    let estimated_delivery = track["dateAndTimes"].as_array().and_then(|dts| {
        dts.iter()
            .find(|dt| dt["type"].as_str() == Some("ESTIMATED_DELIVERY"))
            .and_then(|dt| dt["dateTime"].as_str())
            .map(|s| {
                if s.len() >= 10 {
                    s[..10].to_string()
                } else {
                    s.to_string()
                }
            })
    });

    let mut events: Vec<TrackingEvent> = Vec::new();
    if let Some(scans) = scan_events {
        for scan in scans {
            let ts = scan["date"].as_str().unwrap_or("").to_string();
            let desc = scan["eventDescription"].as_str().unwrap_or("").to_string();
            if desc.is_empty() {
                continue;
            }
            let ev_city = scan["scanLocation"]["city"].as_str().unwrap_or("");
            let ev_state = scan["scanLocation"]["stateOrProvinceCode"]
                .as_str()
                .unwrap_or("");
            let ev_location = if ev_city.is_empty() && ev_state.is_empty() {
                None
            } else {
                Some(
                    format!("{ev_city}, {ev_state}")
                        .trim_matches(',')
                        .trim()
                        .to_string(),
                )
            };
            events.push(TrackingEvent {
                timestamp: ts,
                description: desc,
                location: ev_location,
            });
        }
    }

    Ok(TrackingResult {
        tracking_number: tracking_number.to_string(),
        carrier_code: "fedex".to_string(),
        status,
        location,
        estimated_delivery,
        events,
    })
}

// ── Base64 helper ─────────────────────────────────────────────

fn base64_decode(data: &str) -> Result<Vec<u8>, CarrierProviderError> {
    base64::engine::general_purpose::STANDARD
        .decode(data)
        .map_err(|e| CarrierProviderError::ApiError(format!("FedEx base64 decode error: {e}")))
}

// ── CarrierProvider implementation ───────────────────────────

#[async_trait]
impl CarrierProvider for FedexCarrierProvider {
    fn carrier_code(&self) -> &str {
        "fedex"
    }

    async fn get_rates(
        &self,
        req: &Value,
        config: &Value,
    ) -> Result<Vec<RateQuote>, CarrierProviderError> {
        let client_id = get_client_id(config)?;
        let client_secret = get_client_secret(config)?;
        let account_number = get_account_number(config)?;
        let base_url = get_base_url(config);

        let token = acquire_token(base_url, client_id, client_secret).await?;
        let body = build_rate_request(account_number, req);
        let response = fedex_post(base_url, "/rate/v1/rates/quotes", &token, body).await?;
        parse_rate_response(&response)
    }

    async fn create_label(
        &self,
        req: &Value,
        config: &Value,
    ) -> Result<LabelResult, CarrierProviderError> {
        let client_id = get_client_id(config)?;
        let client_secret = get_client_secret(config)?;
        let account_number = get_account_number(config)?;
        let base_url = get_base_url(config);

        let token = acquire_token(base_url, client_id, client_secret).await?;
        let body = build_ship_request(account_number, req);
        let response = fedex_post(base_url, "/ship/v1/shipments", &token, body).await?;
        parse_ship_response(&response)
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
        let client_id = get_client_id(config)?;
        let client_secret = get_client_secret(config)?;
        let account_number = get_account_number(config)?;
        let base_url = get_base_url(config);

        let token = acquire_token(base_url, client_id, client_secret).await?;
        let body = build_multi_ship_request(account_number, req);
        let response = fedex_post(base_url, "/ship/v1/shipments", &token, body).await?;
        parse_multi_ship_response(&response)
    }

    async fn create_return_label(
        &self,
        req: &Value,
        config: &Value,
    ) -> Result<LabelResult, CarrierProviderError> {
        let client_id = get_client_id(config)?;
        let client_secret = get_client_secret(config)?;
        let account_number = get_account_number(config)?;
        let base_url = get_base_url(config);

        let token = acquire_token(base_url, client_id, client_secret).await?;
        let mut body = build_ship_request(account_number, req);
        // FedEx return shipments: set shipmentType=RETURN in requestedShipment.
        body["requestedShipment"]["shipmentType"] = serde_json::json!("RETURN");
        let response = fedex_post(base_url, "/ship/v1/shipments", &token, body).await?;
        parse_ship_response(&response)
    }

    async fn track(
        &self,
        tracking_number: &str,
        config: &Value,
    ) -> Result<TrackingResult, CarrierProviderError> {
        let client_id = get_client_id(config)?;
        let client_secret = get_client_secret(config)?;
        let base_url = get_base_url(config);

        let token = acquire_token(base_url, client_id, client_secret).await?;
        let body = build_track_request(tracking_number);
        let response = fedex_post(base_url, "/track/v1/trackingnumbers", &token, body).await?;
        parse_track_response(&response, tracking_number)
    }

    async fn fetch_label(
        &self,
        tracking_number: &str,
        config: &Value,
    ) -> Result<LabelPdfResponse, CarrierProviderError> {
        let client_id = get_client_id(config)?;
        let client_secret = get_client_secret(config)?;
        let account_number = get_account_number(config)?;
        let base_url = get_base_url(config);

        let token = acquire_token(base_url, client_id, client_secret).await?;

        let body = serde_json::json!({
            "accountNumber": { "value": account_number },
            "trackingInfo": {
                "trackingNumberInfo": { "trackingNumber": tracking_number }
            },
            "labelResponseOptions": "LABEL",
            "requestedShipment": {
                "labelSpecification": { "imageType": "PDF", "labelStockType": "PAPER_85X11_TOP_HALF_LABEL" }
            }
        });

        let response = fedex_post(base_url, "/ship/v1/shipments/retrieve", &token, body).await?;

        // FedEx retrieve: same structure as ship response.
        let label_result = parse_ship_response(&response).map_err(|_| {
            CarrierProviderError::NotFound(format!(
                "FedEx: label not found or purged for {tracking_number}"
            ))
        })?;

        let pdf_bytes = if label_result.label_data.is_empty() {
            return Err(CarrierProviderError::NotFound(format!(
                "FedEx: no label data returned for {tracking_number}"
            )));
        } else {
            base64_decode(&label_result.label_data)?
        };

        Ok(LabelPdfResponse {
            pdf_bytes,
            content_type: "application/pdf".to_string(),
            carrier_reference: tracking_number.to_string(),
        })
    }
}

// ── Unit tests ────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── dollars_to_minor ─────────────────────────────────────

    #[test]
    fn dollars_to_minor_converts_correctly() {
        assert_eq!(dollars_to_minor(15.50), 1550);
        assert_eq!(dollars_to_minor(35.0), 3500);
        assert_eq!(dollars_to_minor(0.99), 99);
        assert_eq!(dollars_to_minor(0.0), 0);
    }

    // ── parse_transit_time ────────────────────────────────────

    #[test]
    fn parse_transit_time_maps_known_strings() {
        assert_eq!(parse_transit_time("ONE_DAY"), Some(1));
        assert_eq!(parse_transit_time("OVERNIGHT"), Some(1));
        assert_eq!(parse_transit_time("TWO_DAYS"), Some(2));
        assert_eq!(parse_transit_time("FIVE_DAYS"), Some(5));
        assert_eq!(parse_transit_time("SEVEN_DAYS"), Some(7));
    }

    #[test]
    fn parse_transit_time_parses_numeric_strings() {
        assert_eq!(parse_transit_time("3"), Some(3));
        assert_eq!(parse_transit_time("1"), Some(1));
    }

    #[test]
    fn parse_transit_time_returns_none_for_unknown() {
        assert_eq!(parse_transit_time("UNKNOWN"), None);
        assert_eq!(parse_transit_time(""), None);
    }

    // ── build_rate_request ────────────────────────────────────

    #[test]
    fn build_rate_request_uses_defaults_when_req_empty() {
        let body = build_rate_request("TEST_ACCT", &serde_json::json!({}));
        assert_eq!(body["accountNumber"]["value"], "TEST_ACCT");
        let shipment = &body["requestedShipment"];
        assert_eq!(shipment["shipper"]["address"]["postalCode"], "10001");
        assert_eq!(shipment["recipient"]["address"]["postalCode"], "90210");
        let pkg = &shipment["requestedPackageLineItems"][0];
        assert_eq!(pkg["weight"]["value"], 1.0);
        assert_eq!(pkg["weight"]["units"], "LB");
    }

    #[test]
    fn build_rate_request_uses_provided_fields() {
        let req = serde_json::json!({
            "origin_zip": "30301",
            "dest_zip":   "98101",
            "weight_lbs": 5.5,
            "length_in":  8.0,
            "width_in":   6.0,
            "height_in":  4.0,
        });
        let body = build_rate_request("ACCT123", &req);
        let shipment = &body["requestedShipment"];
        assert_eq!(shipment["shipper"]["address"]["postalCode"], "30301");
        assert_eq!(shipment["recipient"]["address"]["postalCode"], "98101");
        let pkg = &shipment["requestedPackageLineItems"][0];
        assert_eq!(pkg["weight"]["value"], 5.5);
        assert_eq!(pkg["dimensions"]["length"], 8.0);
    }

    // ── build_ship_request ────────────────────────────────────

    #[test]
    fn build_ship_request_defaults_to_fedex_ground() {
        let body = build_ship_request("ACCT123", &serde_json::json!({}));
        assert_eq!(body["requestedShipment"]["serviceType"], "FEDEX_GROUND");
    }

    #[test]
    fn build_ship_request_respects_service_type_override() {
        let req = serde_json::json!({"service_type": "PRIORITY_OVERNIGHT"});
        let body = build_ship_request("ACCT123", &req);
        assert_eq!(
            body["requestedShipment"]["serviceType"],
            "PRIORITY_OVERNIGHT"
        );
    }

    #[test]
    fn build_ship_request_embeds_account_number() {
        let body = build_ship_request("ACCT_XYZ", &serde_json::json!({}));
        assert_eq!(body["accountNumber"]["value"], "ACCT_XYZ");
        assert_eq!(
            body["requestedShipment"]["shippingChargesPayment"]["payor"]["responsibleParty"]
                ["accountNumber"]["value"],
            "ACCT_XYZ"
        );
    }

    // ── build_track_request ───────────────────────────────────

    #[test]
    fn build_track_request_embeds_tracking_number() {
        let body = build_track_request("794644790138");
        assert_eq!(
            body["trackingInfo"][0]["trackingNumberInfo"]["trackingNumber"],
            "794644790138"
        );
        assert_eq!(body["includeDetailedScans"], true);
    }

    // ── parse_rate_response ───────────────────────────────────

    #[test]
    fn parse_rate_response_extracts_ground_and_express_quotes() {
        // Simulates FedEx Ground (totalNetCharge at top level, FIVE_DAYS transit)
        // and FedEx Priority Overnight (nested under shipmentRateDetail, ONE_DAY).
        let body = serde_json::json!({
            "output": {
                "rateReplyDetails": [
                    {
                        "serviceType": "FEDEX_GROUND",
                        "serviceName": "FedEx Ground",
                        "ratedShipmentDetails": [{
                            "rateType": "ACCOUNT",
                            "totalNetCharge": 15.50,
                            "currency": "USD",
                            "ratedPackages": [{
                                "packageRateDetail": {
                                    "transitTime": "FIVE_DAYS"
                                }
                            }]
                        }]
                    },
                    {
                        "serviceType": "PRIORITY_OVERNIGHT",
                        "serviceName": "FedEx Priority Overnight",
                        "ratedShipmentDetails": [{
                            "rateType": "LIST",
                            "shipmentRateDetail": {
                                "totalNetCharge": 78.25,
                                "currency": "USD"
                            },
                            "ratedPackages": [{
                                "packageRateDetail": {
                                    "transitTime": "ONE_DAY"
                                }
                            }]
                        }]
                    }
                ]
            }
        });

        let quotes = parse_rate_response(&body).expect("parse_rate_response failed");
        assert_eq!(quotes.len(), 2);

        let ground = &quotes[0];
        assert_eq!(ground.service_level, "FedEx Ground");
        assert_eq!(ground.carrier_code, "fedex");
        assert_eq!(ground.total_charge_minor, 1550);
        assert_eq!(ground.currency, "USD");
        assert_eq!(ground.estimated_days, Some(5));

        let overnight = &quotes[1];
        assert_eq!(overnight.service_level, "FedEx Priority Overnight");
        assert_eq!(overnight.total_charge_minor, 7825);
        assert_eq!(overnight.estimated_days, Some(1));
    }

    #[test]
    fn parse_rate_response_prefers_account_rate_over_list() {
        let body = serde_json::json!({
            "output": {
                "rateReplyDetails": [{
                    "serviceType": "FEDEX_GROUND",
                    "serviceName": "FedEx Ground",
                    "ratedShipmentDetails": [
                        {
                            "rateType": "LIST",
                            "totalNetCharge": 25.00,
                            "currency": "USD"
                        },
                        {
                            "rateType": "ACCOUNT",
                            "totalNetCharge": 18.50,
                            "currency": "USD"
                        }
                    ]
                }]
            }
        });

        let quotes = parse_rate_response(&body).expect("parse failed");
        assert_eq!(quotes.len(), 1);
        assert_eq!(
            quotes[0].total_charge_minor, 1850,
            "should use ACCOUNT rate"
        );
    }

    #[test]
    fn parse_rate_response_falls_back_to_service_type_when_no_service_name() {
        let body = serde_json::json!({
            "output": {
                "rateReplyDetails": [{
                    "serviceType": "FEDEX_2_DAY",
                    "ratedShipmentDetails": [{
                        "rateType": "ACCOUNT",
                        "totalNetCharge": 30.00,
                        "currency": "USD"
                    }]
                }]
            }
        });

        let quotes = parse_rate_response(&body).expect("parse failed");
        assert_eq!(quotes[0].service_level, "FEDEX_2_DAY");
    }

    #[test]
    fn parse_rate_response_returns_error_when_no_details() {
        let body = serde_json::json!({"output": {"rateReplyDetails": []}});
        let result = parse_rate_response(&body);
        assert!(result.is_err());
    }

    #[test]
    fn parse_rate_response_returns_error_when_field_missing() {
        let body = serde_json::json!({"output": {}});
        let result = parse_rate_response(&body);
        assert!(result.is_err());
    }

    // ── parse_ship_response ───────────────────────────────────

    #[test]
    fn parse_ship_response_extracts_tracking_and_label() {
        let body = serde_json::json!({
            "output": {
                "transactionShipments": [{
                    "masterTrackingNumber": "794644790138",
                    "serviceType": "FEDEX_GROUND",
                    "pieceResponses": [{
                        "trackingNumber": "794644790138",
                        "packageDocuments": [{
                            "contentType": "LABEL",
                            "encodedLabel": "JVBERi0xLjQ="
                        }]
                    }]
                }]
            }
        });

        let result = parse_ship_response(&body).expect("parse failed");
        assert_eq!(result.tracking_number, "794644790138");
        assert_eq!(result.label_data, "JVBERi0xLjQ=");
        assert_eq!(result.label_format, "pdf");
        assert_eq!(result.carrier_code, "fedex");
    }

    #[test]
    fn parse_ship_response_falls_back_to_master_tracking() {
        // Piece tracking number absent — fall back to masterTrackingNumber
        let body = serde_json::json!({
            "output": {
                "transactionShipments": [{
                    "masterTrackingNumber": "794644790999",
                    "pieceResponses": [{
                        "packageDocuments": []
                    }]
                }]
            }
        });

        let result = parse_ship_response(&body).expect("parse failed");
        assert_eq!(result.tracking_number, "794644790999");
    }

    #[test]
    fn parse_ship_response_returns_error_when_no_shipments() {
        let body = serde_json::json!({"output": {"transactionShipments": []}});
        assert!(parse_ship_response(&body).is_err());
    }

    // ── parse_track_response ──────────────────────────────────

    #[test]
    fn parse_track_response_extracts_status_location_and_events() {
        let body = serde_json::json!({
            "output": {
                "completeTrackResults": [{
                    "trackingNumber": "794644790138",
                    "trackResults": [{
                        "latestStatusDetail": {
                            "description": "In transit"
                        },
                        "dateAndTimes": [{
                            "type": "ESTIMATED_DELIVERY",
                            "dateTime": "2026-04-12T00:00:00"
                        }],
                        "scanEvents": [{
                            "date": "2026-04-09T10:00:00-05:00",
                            "eventType": "PU",
                            "eventDescription": "Picked up",
                            "scanLocation": {
                                "city": "Memphis",
                                "stateOrProvinceCode": "TN"
                            }
                        }]
                    }]
                }]
            }
        });

        let result = parse_track_response(&body, "794644790138").expect("parse failed");
        assert_eq!(result.tracking_number, "794644790138");
        assert_eq!(result.carrier_code, "fedex");
        assert_eq!(result.status, "In transit");
        assert_eq!(result.location, Some("Memphis, TN".to_string()));
        assert_eq!(result.estimated_delivery, Some("2026-04-12".to_string()));
        assert_eq!(result.events.len(), 1);
        assert_eq!(result.events[0].description, "Picked up");
        assert_eq!(result.events[0].location, Some("Memphis, TN".to_string()));
    }

    #[test]
    fn parse_track_response_returns_error_on_tracking_error() {
        let body = serde_json::json!({
            "output": {
                "completeTrackResults": [{
                    "trackingNumber": "INVALID",
                    "trackResults": [{
                        "error": {
                            "code": "TRACKING.TRACKINGNUMBER.NOTFOUND",
                            "message": "Tracking number cannot be found."
                        }
                    }]
                }]
            }
        });

        let result = parse_track_response(&body, "INVALID");
        assert!(result.is_err());
        let msg = format!("{}", result.unwrap_err());
        assert!(msg.contains("Tracking number cannot be found"));
    }

    #[test]
    fn parse_track_response_falls_back_to_status_by_locale() {
        let body = serde_json::json!({
            "output": {
                "completeTrackResults": [{
                    "trackResults": [{
                        "latestStatusDetail": {
                            "statusByLocale": "Delivered"
                        },
                        "scanEvents": []
                    }]
                }]
            }
        });

        let result = parse_track_response(&body, "794644790138").expect("parse failed");
        assert_eq!(result.status, "Delivered");
    }

    // ── Credential helpers ────────────────────────────────────

    #[test]
    fn missing_client_id_returns_credentials_error() {
        let empty = serde_json::json!({});
        let result = get_client_id(&empty);
        assert!(matches!(
            result,
            Err(CarrierProviderError::CredentialsError(_))
        ));
    }

    #[test]
    fn missing_client_secret_returns_credentials_error() {
        let empty = serde_json::json!({});
        let result = get_client_secret(&empty);
        assert!(matches!(
            result,
            Err(CarrierProviderError::CredentialsError(_))
        ));
    }

    #[test]
    fn missing_account_number_returns_credentials_error() {
        let empty = serde_json::json!({});
        let result = get_account_number(&empty);
        assert!(matches!(
            result,
            Err(CarrierProviderError::CredentialsError(_))
        ));
    }

    #[test]
    fn get_base_url_defaults_to_production() {
        assert_eq!(get_base_url(&serde_json::json!({})), FEDEX_PRODUCTION_URL);
    }

    #[test]
    fn get_base_url_uses_override_when_present() {
        let config = serde_json::json!({"base_url": "https://apis-sandbox.fedex.com"});
        assert_eq!(get_base_url(&config), "https://apis-sandbox.fedex.com");
    }

    // ── Token cache ───────────────────────────────────────────

    #[test]
    fn token_cache_stores_and_retrieves_valid_token() {
        let client_id = "test-cache-client-id-unit";
        store_cached_token(client_id, "test-token-value".to_string(), 3600);
        let retrieved = get_cached_token(client_id);
        assert_eq!(retrieved, Some("test-token-value".to_string()));
    }

    #[test]
    fn token_cache_returns_none_for_expired_token() {
        let client_id = "test-cache-expired-unit";
        // Store with 0 TTL — expires immediately (or within 60 s buffer)
        store_cached_token(client_id, "expired-token".to_string(), 0);
        // TTL = 0.saturating_sub(60) = 0 → expires_at = now + 0 s → already expired
        let retrieved = get_cached_token(client_id);
        assert!(
            retrieved.is_none(),
            "expected None for immediately-expired token"
        );
    }
}
