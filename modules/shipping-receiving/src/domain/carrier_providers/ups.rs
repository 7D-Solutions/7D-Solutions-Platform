//! UPS carrier provider.
//!
//! Implements `CarrierProvider` for UPS via the OAuth2 REST APIs:
//! - Rate quotes:     UPS Rating API v2205
//! - Label creation:  UPS Shipping API v2201
//! - Tracking:        UPS Track API v1
//!
//! ## Config JSON
//! ```json
//! {
//!   "client_id":      "your-ups-client-id",
//!   "client_secret":  "your-ups-client-secret",
//!   "account_number": "your-ups-account-number",
//!   "base_url":       "https://wwwcie.ups.com"
//! }
//! ```
//! `base_url` is optional — defaults to the UPS CIE (sandbox) endpoint.
//!
//! ## Token cache
//! Bearer tokens are cached per `UpsCarrierProvider` instance behind a
//! `tokio::sync::RwLock`. The double-checked locking pattern ensures that
//! concurrent calls within the same instance never issue duplicate OAuth
//! requests. Tokens are pre-emptively refreshed 60 seconds before expiry.

use std::time::{Duration, Instant};

use async_trait::async_trait;
use reqwest::Client;
use serde_json::Value;
use tokio::sync::RwLock;

use super::{
    CarrierProvider, CarrierProviderError, LabelResult, RateQuote, TrackingEvent, TrackingResult,
};

const UPS_SANDBOX_URL: &str = "https://wwwcie.ups.com";
const TOKEN_EXPIRY_BUFFER_SECS: u64 = 60;

pub struct UpsCarrierProvider {
    token_cache: RwLock<Option<(String, Instant)>>,
}

impl UpsCarrierProvider {
    pub fn new() -> Self {
        Self {
            token_cache: RwLock::new(None),
        }
    }
}

// ── Config helpers ─────────────────────────────────────────────

fn get_client_id(config: &Value) -> Result<&str, CarrierProviderError> {
    config["client_id"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            CarrierProviderError::CredentialsError(
                "UPS config missing required field 'client_id'".to_string(),
            )
        })
}

fn get_client_secret(config: &Value) -> Result<&str, CarrierProviderError> {
    config["client_secret"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            CarrierProviderError::CredentialsError(
                "UPS config missing required field 'client_secret'".to_string(),
            )
        })
}

fn get_account_number(config: &Value) -> Result<&str, CarrierProviderError> {
    config["account_number"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            CarrierProviderError::CredentialsError(
                "UPS config missing required field 'account_number'".to_string(),
            )
        })
}

fn get_base_url(config: &Value) -> &str {
    config["base_url"]
        .as_str()
        .filter(|s| !s.is_empty())
        .unwrap_or(UPS_SANDBOX_URL)
}

// ── Error extraction ───────────────────────────────────────────

/// Extract a human-readable error message from a UPS API error response.
///
/// UPS REST error format:
/// `{ "response": { "errors": [{ "code": "...", "message": "..." }] } }`
pub(crate) fn extract_ups_error(json: &Value) -> String {
    json["response"]["errors"]
        .as_array()
        .and_then(|a| a.first())
        .map(|e| {
            let code = e["code"].as_str().unwrap_or("UNKNOWN");
            let msg = e["message"].as_str().unwrap_or("unknown error");
            format!("{code}: {msg}")
        })
        .unwrap_or_else(|| "unknown UPS API error".to_string())
}

// ── OAuth2 token management ────────────────────────────────────

impl UpsCarrierProvider {
    /// Return a valid bearer token, refreshing only when necessary.
    ///
    /// Uses double-checked locking: fast path under read lock, slow path
    /// under write lock. Only one task ever calls the OAuth endpoint at a
    /// time for a given provider instance.
    async fn get_token(
        &self,
        client: &Client,
        config: &Value,
    ) -> Result<String, CarrierProviderError> {
        {
            let guard = self.token_cache.read().await;
            if let Some((token, expiry)) = guard.as_ref() {
                if Instant::now() < *expiry {
                    return Ok(token.clone());
                }
            }
        }

        let mut guard = self.token_cache.write().await;
        if let Some((token, expiry)) = guard.as_ref() {
            if Instant::now() < *expiry {
                return Ok(token.clone());
            }
        }

        let (token, expires_in) = fetch_oauth_token(client, config).await?;
        let expiry = Instant::now()
            + Duration::from_secs(expires_in.saturating_sub(TOKEN_EXPIRY_BUFFER_SECS));
        *guard = Some((token.clone(), expiry));
        Ok(token)
    }
}

async fn fetch_oauth_token(
    client: &Client,
    config: &Value,
) -> Result<(String, u64), CarrierProviderError> {
    let client_id = get_client_id(config)?;
    let client_secret = get_client_secret(config)?;
    let base_url = get_base_url(config);
    let url = format!("{base_url}/security/v1/oauth/token");

    let resp = client
        .post(&url)
        .basic_auth(client_id, Some(client_secret))
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body("grant_type=client_credentials")
        .send()
        .await
        .map_err(|e| CarrierProviderError::ApiError(format!("UPS OAuth HTTP error: {e}")))?;

    let status = resp.status();
    let body = resp.text().await.map_err(|e| {
        CarrierProviderError::ApiError(format!("UPS OAuth response read error: {e}"))
    })?;

    let json: Value = serde_json::from_str(&body).unwrap_or_else(|_| serde_json::json!({}));

    if !status.is_success() {
        let desc = extract_ups_error(&json);
        return Err(CarrierProviderError::ApiError(format!(
            "UPS OAuth error (HTTP {status}): {desc}"
        )));
    }

    let token = json["access_token"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            CarrierProviderError::ApiError("UPS OAuth response missing access_token".to_string())
        })?
        .to_string();

    let expires_in = json["expires_in"].as_u64().unwrap_or(14399);
    Ok((token, expires_in))
}

// ── HTTP helpers ───────────────────────────────────────────────

async fn ups_post(
    client: &Client,
    token: &str,
    url: &str,
    body: &Value,
) -> Result<Value, CarrierProviderError> {
    let resp = client
        .post(url)
        .bearer_auth(token)
        .header("transId", uuid::Uuid::new_v4().to_string())
        .header("transactionSrc", "7D-Solutions")
        .json(body)
        .send()
        .await
        .map_err(|e| CarrierProviderError::ApiError(format!("UPS HTTP error: {e}")))?;

    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| CarrierProviderError::ApiError(format!("UPS response read error: {e}")))?;

    let json: Value = serde_json::from_str(&text).unwrap_or_else(|_| serde_json::json!({}));

    if !status.is_success() {
        let desc = extract_ups_error(&json);
        return Err(CarrierProviderError::ApiError(format!(
            "UPS API error (HTTP {status}): {desc}"
        )));
    }

    Ok(json)
}

async fn ups_get(client: &Client, token: &str, url: &str) -> Result<Value, CarrierProviderError> {
    let resp = client
        .get(url)
        .bearer_auth(token)
        .header("transId", uuid::Uuid::new_v4().to_string())
        .header("transactionSrc", "7D-Solutions")
        .send()
        .await
        .map_err(|e| CarrierProviderError::ApiError(format!("UPS HTTP error: {e}")))?;

    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| CarrierProviderError::ApiError(format!("UPS response read error: {e}")))?;

    let json: Value = serde_json::from_str(&text).unwrap_or_else(|_| serde_json::json!({}));

    if !status.is_success() {
        let desc = extract_ups_error(&json);
        return Err(CarrierProviderError::ApiError(format!(
            "UPS API error (HTTP {status}): {desc}"
        )));
    }

    Ok(json)
}

// ── Rate request builder ───────────────────────────────────────

pub(crate) fn build_rate_request(account_number: &str, req: &Value) -> Value {
    let origin_zip = req["origin_zip"].as_str().unwrap_or("10001");
    let dest_zip = req["dest_zip"].as_str().unwrap_or("90210");
    let origin_state = req["origin_state"].as_str().unwrap_or("NY");
    let dest_state = req["dest_state"].as_str().unwrap_or("CA");
    let origin_city = req["origin_city"].as_str().unwrap_or("New York");
    let dest_city = req["dest_city"].as_str().unwrap_or("Los Angeles");
    let weight = req["weight_lbs"].as_f64().unwrap_or(1.0).to_string();
    let length = req["length_in"].as_f64().unwrap_or(12.0).to_string();
    let width = req["width_in"].as_f64().unwrap_or(12.0).to_string();
    let height = req["height_in"].as_f64().unwrap_or(12.0).to_string();

    serde_json::json!({
        "RateRequest": {
            "Request": {
                "RequestOption": "Shop",
                "TransactionReference": { "CustomerContext": "Rate shopping" }
            },
            "Shipment": {
                "Shipper": {
                    "Name": "Shipper",
                    "ShipperNumber": account_number,
                    "Address": {
                        "AddressLine": ["123 Main St"],
                        "City": origin_city,
                        "StateProvinceCode": origin_state,
                        "PostalCode": origin_zip,
                        "CountryCode": "US"
                    }
                },
                "ShipTo": {
                    "Name": "Recipient",
                    "Address": {
                        "AddressLine": ["456 Sunset Blvd"],
                        "City": dest_city,
                        "StateProvinceCode": dest_state,
                        "PostalCode": dest_zip,
                        "CountryCode": "US"
                    }
                },
                "ShipFrom": {
                    "Name": "Shipper",
                    "Address": {
                        "AddressLine": ["123 Main St"],
                        "City": origin_city,
                        "StateProvinceCode": origin_state,
                        "PostalCode": origin_zip,
                        "CountryCode": "US"
                    }
                },
                "Package": {
                    "PackagingType": { "Code": "02", "Description": "Package" },
                    "Dimensions": {
                        "UnitOfMeasurement": { "Code": "IN", "Description": "Inches" },
                        "Length": length,
                        "Width": width,
                        "Height": height
                    },
                    "PackageWeight": {
                        "UnitOfMeasurement": { "Code": "LBS", "Description": "Pounds" },
                        "Weight": weight
                    }
                }
            }
        }
    })
}

// ── Rate response parser ───────────────────────────────────────

pub(crate) fn parse_rate_response(json: &Value) -> Result<Vec<RateQuote>, CarrierProviderError> {
    let rated = json["RateResponse"]["RatedShipment"]
        .as_array()
        .ok_or_else(|| {
            CarrierProviderError::ApiError(
                "UPS Rate response missing RatedShipment array".to_string(),
            )
        })?;

    if rated.is_empty() {
        return Err(CarrierProviderError::ApiError(
            "UPS Rate response returned no shipment options".to_string(),
        ));
    }

    let quotes = rated
        .iter()
        .map(|s| {
            let code = s["Service"]["Code"].as_str().unwrap_or("UNK");
            let service_level = ups_service_name(code);

            let charge = s["NegotiatedRateCharges"]["TotalCharge"]["MonetaryValue"]
                .as_str()
                .or_else(|| s["TotalCharges"]["MonetaryValue"].as_str())
                .unwrap_or("0.00");

            let currency = s["TotalCharges"]["CurrencyCode"].as_str().unwrap_or("USD");

            let charge_cents = (charge.parse::<f64>().unwrap_or(0.0) * 100.0).round() as i64;

            let estimated_days = s["GuaranteedDelivery"]["BusinessDaysInTransit"]
                .as_str()
                .and_then(|v| v.parse::<i32>().ok());

            RateQuote {
                service_level,
                carrier_code: "ups".to_string(),
                total_charge_minor: charge_cents,
                currency: currency.to_string(),
                estimated_days,
            }
        })
        .collect();

    Ok(quotes)
}

pub(crate) fn ups_service_name(code: &str) -> String {
    match code {
        "01" => "UPS Next Day Air",
        "02" => "UPS 2nd Day Air",
        "03" => "UPS Ground",
        "07" => "UPS Worldwide Express",
        "08" => "UPS Worldwide Expedited",
        "11" => "UPS Standard",
        "12" => "UPS 3 Day Select",
        "13" => "UPS Next Day Air Saver",
        "14" => "UPS Next Day Air Early",
        "54" => "UPS Worldwide Express Plus",
        "59" => "UPS 2nd Day Air A.M.",
        "65" => "UPS Worldwide Saver",
        other => return format!("UPS Service {other}"),
    }
    .to_string()
}

// ── Ship request builder ───────────────────────────────────────

pub(crate) fn build_ship_request(account_number: &str, req: &Value) -> Value {
    let from_name = req["from_name"].as_str().unwrap_or("Sender");
    let from_addr = req["from_address"].as_str().unwrap_or("123 Main St");
    let from_city = req["from_city"].as_str().unwrap_or("New York");
    let from_state = req["from_state"].as_str().unwrap_or("NY");
    let from_zip = req["from_zip"].as_str().unwrap_or("10001");
    let to_name = req["to_name"].as_str().unwrap_or("Recipient");
    let to_addr = req["to_address"].as_str().unwrap_or("456 Sunset Blvd");
    let to_city = req["to_city"].as_str().unwrap_or("Los Angeles");
    let to_state = req["to_state"].as_str().unwrap_or("CA");
    let to_zip = req["to_zip"].as_str().unwrap_or("90210");
    let weight = req["weight_lbs"].as_f64().unwrap_or(1.0).to_string();
    let length = req["length_in"].as_f64().unwrap_or(12.0).to_string();
    let width = req["width_in"].as_f64().unwrap_or(12.0).to_string();
    let height = req["height_in"].as_f64().unwrap_or(12.0).to_string();

    serde_json::json!({
        "ShipmentRequest": {
            "Request": {
                "RequestOption": "nonvalidate",
                "TransactionReference": { "CustomerContext": "Label creation" }
            },
            "Shipment": {
                "Description": "Shipment",
                "Shipper": {
                    "Name": from_name,
                    "AttentionName": from_name,
                    "ShipperNumber": account_number,
                    "Phone": { "Number": "0000000000" },
                    "Address": {
                        "AddressLine": [from_addr],
                        "City": from_city,
                        "StateProvinceCode": from_state,
                        "PostalCode": from_zip,
                        "CountryCode": "US"
                    }
                },
                "ShipTo": {
                    "Name": to_name,
                    "AttentionName": to_name,
                    "Phone": { "Number": "0000000000" },
                    "Address": {
                        "AddressLine": [to_addr],
                        "City": to_city,
                        "StateProvinceCode": to_state,
                        "PostalCode": to_zip,
                        "CountryCode": "US"
                    }
                },
                "ShipFrom": {
                    "Name": from_name,
                    "AttentionName": from_name,
                    "Phone": { "Number": "0000000000" },
                    "Address": {
                        "AddressLine": [from_addr],
                        "City": from_city,
                        "StateProvinceCode": from_state,
                        "PostalCode": from_zip,
                        "CountryCode": "US"
                    }
                },
                "PaymentInformation": {
                    "ShipmentCharge": {
                        "Type": "01",
                        "BillShipper": { "AccountNumber": account_number }
                    }
                },
                "Service": { "Code": "03", "Description": "Ground" },
                "Package": {
                    "Description": "Package",
                    "PackagingType": { "Code": "02", "Description": "Package" },
                    "Dimensions": {
                        "UnitOfMeasurement": { "Code": "IN" },
                        "Length": length,
                        "Width": width,
                        "Height": height
                    },
                    "PackageWeight": {
                        "UnitOfMeasurement": { "Code": "LBS" },
                        "Weight": weight
                    }
                }
            },
            "LabelSpecification": {
                "LabelImageFormat": { "Code": "GIF", "Description": "GIF" },
                "HTTPUserAgent": "Mozilla/4.5"
            }
        }
    })
}

// ── Ship response parser ───────────────────────────────────────

pub(crate) fn parse_ship_response(json: &Value) -> Result<LabelResult, CarrierProviderError> {
    let results = &json["ShipmentResponse"]["ShipmentResults"];
    let pkg = &results["PackageResults"];

    // PackageResults is an object for single-package, array for multi-package.
    let (tracking_number, label_image, image_format) = if pkg.is_object() {
        let tn = pkg["TrackingNumber"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                CarrierProviderError::ApiError(
                    "UPS Ship response missing TrackingNumber".to_string(),
                )
            })?;
        let img = pkg["ShippingLabel"]["GraphicImage"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                CarrierProviderError::ApiError(
                    "UPS Ship response missing ShippingLabel.GraphicImage".to_string(),
                )
            })?;
        let fmt = pkg["ShippingLabel"]["ImageFormat"]["Code"]
            .as_str()
            .unwrap_or("GIF");
        (tn, img, fmt)
    } else if let Some(arr) = pkg.as_array() {
        let first = arr.first().ok_or_else(|| {
            CarrierProviderError::ApiError(
                "UPS Ship response PackageResults array is empty".to_string(),
            )
        })?;
        let tn = first["TrackingNumber"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                CarrierProviderError::ApiError(
                    "UPS Ship response missing TrackingNumber".to_string(),
                )
            })?;
        let img = first["ShippingLabel"]["GraphicImage"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                CarrierProviderError::ApiError(
                    "UPS Ship response missing ShippingLabel.GraphicImage".to_string(),
                )
            })?;
        let fmt = first["ShippingLabel"]["ImageFormat"]["Code"]
            .as_str()
            .unwrap_or("GIF");
        (tn, img, fmt)
    } else {
        return Err(CarrierProviderError::ApiError(
            "UPS Ship response missing PackageResults".to_string(),
        ));
    };

    Ok(LabelResult {
        tracking_number: tracking_number.to_string(),
        label_format: image_format.to_lowercase(),
        label_data: label_image.to_string(),
        carrier_code: "ups".to_string(),
    })
}

// ── Track response parser ──────────────────────────────────────

pub(crate) fn parse_track_response(
    json: &Value,
    tracking_number: &str,
) -> Result<TrackingResult, CarrierProviderError> {
    let shipments = json["trackResponse"]["shipment"]
        .as_array()
        .ok_or_else(|| {
            CarrierProviderError::ApiError("UPS Track response missing shipment array".to_string())
        })?;

    let shipment = shipments.first().ok_or_else(|| {
        CarrierProviderError::ApiError("UPS Track response has empty shipment array".to_string())
    })?;

    let packages = shipment["package"].as_array().ok_or_else(|| {
        CarrierProviderError::ApiError("UPS Track response missing package array".to_string())
    })?;

    let pkg = packages.first().ok_or_else(|| {
        CarrierProviderError::ApiError("UPS Track response has empty package array".to_string())
    })?;

    let activities = pkg["activity"].as_array().cloned().unwrap_or_default();

    let status = activities
        .first()
        .and_then(|a| a["status"]["description"].as_str())
        .unwrap_or("UNKNOWN")
        .to_string();

    let location = activities.first().and_then(|a| {
        let city = a["location"]["address"]["city"].as_str().unwrap_or("");
        let state = a["location"]["address"]["stateProvince"]
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

    let estimated_delivery = pkg["deliveryDate"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(|d| d["date"].as_str())
        .map(|s| s.to_string());

    let events: Vec<TrackingEvent> = activities
        .iter()
        .map(|a| {
            let date = a["date"].as_str().unwrap_or("");
            let time = a["time"].as_str().unwrap_or("");
            let timestamp = format!("{date} {time}").trim().to_string();
            let description = a["status"]["description"]
                .as_str()
                .unwrap_or("Unknown")
                .to_string();
            let city = a["location"]["address"]["city"].as_str().unwrap_or("");
            let state = a["location"]["address"]["stateProvince"]
                .as_str()
                .unwrap_or("");
            let evt_location = if city.is_empty() && state.is_empty() {
                None
            } else {
                Some(
                    format!("{city}, {state}")
                        .trim_matches(',')
                        .trim()
                        .to_string(),
                )
            };
            TrackingEvent {
                timestamp,
                description,
                location: evt_location,
            }
        })
        .collect();

    Ok(TrackingResult {
        tracking_number: tracking_number.to_string(),
        carrier_code: "ups".to_string(),
        status,
        location,
        estimated_delivery,
        events,
    })
}

// ── CarrierProvider implementation ────────────────────────────

#[async_trait]
impl CarrierProvider for UpsCarrierProvider {
    fn carrier_code(&self) -> &str {
        "ups"
    }

    async fn get_rates(
        &self,
        req: &Value,
        config: &Value,
    ) -> Result<Vec<RateQuote>, CarrierProviderError> {
        let account_number = get_account_number(config)?.to_string();
        let base_url = get_base_url(config).to_string();
        let client = Client::new();
        let token = self.get_token(&client, config).await?;
        let url = format!("{base_url}/api/rating/v2205/Rate");
        let body = build_rate_request(&account_number, req);
        let json = ups_post(&client, &token, &url, &body).await?;
        parse_rate_response(&json)
    }

    async fn create_label(
        &self,
        req: &Value,
        config: &Value,
    ) -> Result<LabelResult, CarrierProviderError> {
        let account_number = get_account_number(config)?.to_string();
        let base_url = get_base_url(config).to_string();
        let client = Client::new();
        let token = self.get_token(&client, config).await?;
        let url = format!("{base_url}/api/shipments/v2201/ship");
        let body = build_ship_request(&account_number, req);
        let json = ups_post(&client, &token, &url, &body).await?;
        parse_ship_response(&json)
    }

    async fn track(
        &self,
        tracking_number: &str,
        config: &Value,
    ) -> Result<TrackingResult, CarrierProviderError> {
        let base_url = get_base_url(config).to_string();
        let client = Client::new();
        let token = self.get_token(&client, config).await?;
        let url = format!(
            "{base_url}/api/track/v1/details/{tracking_number}?locale=en_US&returnSignature=false"
        );
        let json = ups_get(&client, &token, &url).await?;
        parse_track_response(&json, tracking_number)
    }
}

// ── Unit tests ─────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Config helpers ─────────────────────────────────────────

    #[test]
    fn missing_client_id_returns_credentials_error() {
        let config = serde_json::json!({});
        assert!(matches!(
            get_client_id(&config),
            Err(CarrierProviderError::CredentialsError(_))
        ));
    }

    #[test]
    fn missing_client_secret_returns_credentials_error() {
        let config = serde_json::json!({ "client_id": "x" });
        assert!(matches!(
            get_client_secret(&config),
            Err(CarrierProviderError::CredentialsError(_))
        ));
    }

    #[test]
    fn missing_account_number_returns_credentials_error() {
        let config = serde_json::json!({ "client_id": "x", "client_secret": "y" });
        assert!(matches!(
            get_account_number(&config),
            Err(CarrierProviderError::CredentialsError(_))
        ));
    }

    #[test]
    fn get_base_url_defaults_to_sandbox() {
        let config = serde_json::json!({});
        assert_eq!(get_base_url(&config), UPS_SANDBOX_URL);
    }

    #[test]
    fn get_base_url_uses_provided_value() {
        let config = serde_json::json!({ "base_url": "https://onlinetools.ups.com" });
        assert_eq!(get_base_url(&config), "https://onlinetools.ups.com");
    }

    // ── Service name mapping ───────────────────────────────────

    #[test]
    fn service_name_maps_known_codes() {
        assert_eq!(ups_service_name("03"), "UPS Ground");
        assert_eq!(ups_service_name("01"), "UPS Next Day Air");
        assert_eq!(ups_service_name("02"), "UPS 2nd Day Air");
        assert_eq!(ups_service_name("12"), "UPS 3 Day Select");
    }

    #[test]
    fn service_name_falls_back_for_unknown_code() {
        assert_eq!(ups_service_name("99"), "UPS Service 99");
    }

    // ── Error extraction ───────────────────────────────────────

    #[test]
    fn extract_ups_error_parses_standard_format() {
        let json = serde_json::json!({
            "response": {
                "errors": [{ "code": "250001", "message": "Access License number is invalid" }]
            }
        });
        let msg = extract_ups_error(&json);
        assert!(msg.contains("250001"));
        assert!(msg.contains("Access License number is invalid"));
    }

    #[test]
    fn extract_ups_error_returns_fallback_on_empty() {
        let json = serde_json::json!({});
        assert_eq!(extract_ups_error(&json), "unknown UPS API error");
    }

    // ── Rate request builder ───────────────────────────────────

    #[test]
    fn build_rate_request_uses_req_fields() {
        let req = serde_json::json!({
            "origin_zip": "10001",
            "dest_zip": "90210",
            "weight_lbs": 5.0,
        });
        let body = build_rate_request("TEST123", &req);
        assert_eq!(
            body["RateRequest"]["Shipment"]["Shipper"]["ShipperNumber"],
            "TEST123"
        );
        assert_eq!(
            body["RateRequest"]["Shipment"]["Package"]["PackageWeight"]["Weight"],
            "5"
        );
        assert_eq!(
            body["RateRequest"]["Shipment"]["Shipper"]["Address"]["PostalCode"],
            "10001"
        );
        assert_eq!(
            body["RateRequest"]["Shipment"]["ShipTo"]["Address"]["PostalCode"],
            "90210"
        );
    }

    #[test]
    fn build_rate_request_applies_defaults() {
        let body = build_rate_request("ACCT", &serde_json::json!({}));
        assert_eq!(body["RateRequest"]["Request"]["RequestOption"], "Shop");
        assert_eq!(
            body["RateRequest"]["Shipment"]["Package"]["PackageWeight"]["Weight"],
            "1"
        );
    }

    // ── Rate response parser ───────────────────────────────────

    #[test]
    fn parse_rate_response_extracts_quotes() {
        let json = serde_json::json!({
            "RateResponse": {
                "RatedShipment": [
                    {
                        "Service": { "Code": "03" },
                        "TotalCharges": { "MonetaryValue": "15.50", "CurrencyCode": "USD" }
                    },
                    {
                        "Service": { "Code": "01" },
                        "TotalCharges": { "MonetaryValue": "45.00", "CurrencyCode": "USD" },
                        "GuaranteedDelivery": { "BusinessDaysInTransit": "1" }
                    }
                ]
            }
        });
        let quotes = parse_rate_response(&json).expect("parse failed");
        assert_eq!(quotes.len(), 2);
        assert_eq!(quotes[0].service_level, "UPS Ground");
        assert_eq!(quotes[0].total_charge_minor, 1550);
        assert_eq!(quotes[0].carrier_code, "ups");
        assert_eq!(quotes[0].currency, "USD");
        assert_eq!(quotes[0].estimated_days, None);
        assert_eq!(quotes[1].service_level, "UPS Next Day Air");
        assert_eq!(quotes[1].total_charge_minor, 4500);
        assert_eq!(quotes[1].estimated_days, Some(1));
    }

    #[test]
    fn parse_rate_response_prefers_negotiated_charges() {
        let json = serde_json::json!({
            "RateResponse": {
                "RatedShipment": [{
                    "Service": { "Code": "03" },
                    "TotalCharges": { "MonetaryValue": "20.00", "CurrencyCode": "USD" },
                    "NegotiatedRateCharges": {
                        "TotalCharge": { "MonetaryValue": "12.00" }
                    }
                }]
            }
        });
        let quotes = parse_rate_response(&json).expect("parse failed");
        assert_eq!(quotes[0].total_charge_minor, 1200);
    }

    #[test]
    fn parse_rate_response_errors_on_missing_array() {
        let json = serde_json::json!({ "RateResponse": {} });
        assert!(parse_rate_response(&json).is_err());
    }

    #[test]
    fn parse_rate_response_errors_on_empty_array() {
        let json = serde_json::json!({ "RateResponse": { "RatedShipment": [] } });
        assert!(parse_rate_response(&json).is_err());
    }

    // ── Ship response parser ───────────────────────────────────

    #[test]
    fn parse_ship_response_extracts_tracking_and_label_object() {
        let json = serde_json::json!({
            "ShipmentResponse": {
                "ShipmentResults": {
                    "PackageResults": {
                        "TrackingNumber": "1Z12345E0205271688",
                        "ShippingLabel": {
                            "ImageFormat": { "Code": "GIF" },
                            "GraphicImage": "R0lGODlh"
                        }
                    }
                }
            }
        });
        let result = parse_ship_response(&json).expect("parse failed");
        assert_eq!(result.tracking_number, "1Z12345E0205271688");
        assert_eq!(result.label_data, "R0lGODlh");
        assert_eq!(result.label_format, "gif");
        assert_eq!(result.carrier_code, "ups");
    }

    #[test]
    fn parse_ship_response_extracts_from_array() {
        let json = serde_json::json!({
            "ShipmentResponse": {
                "ShipmentResults": {
                    "PackageResults": [{
                        "TrackingNumber": "1Z12345E0205271688",
                        "ShippingLabel": {
                            "ImageFormat": { "Code": "GIF" },
                            "GraphicImage": "R0lGODlh"
                        }
                    }]
                }
            }
        });
        let result = parse_ship_response(&json).expect("parse failed");
        assert_eq!(result.tracking_number, "1Z12345E0205271688");
    }

    #[test]
    fn parse_ship_response_errors_on_missing_results() {
        let json = serde_json::json!({ "ShipmentResponse": { "ShipmentResults": {} } });
        assert!(parse_ship_response(&json).is_err());
    }

    // ── Track response parser ──────────────────────────────────

    #[test]
    fn parse_track_response_extracts_status_and_events() {
        let json = serde_json::json!({
            "trackResponse": {
                "shipment": [{
                    "package": [{
                        "trackingNumber": "1Z12345E0291980793",
                        "activity": [
                            {
                                "date": "20260408",
                                "time": "143000",
                                "status": { "description": "Delivered" },
                                "location": { "address": { "city": "Beverly Hills", "stateProvince": "CA" } }
                            },
                            {
                                "date": "20260408",
                                "time": "080000",
                                "status": { "description": "Out For Delivery Today" },
                                "location": { "address": { "city": "Los Angeles", "stateProvince": "CA" } }
                            }
                        ],
                        "deliveryDate": [{ "date": "20260408" }]
                    }]
                }]
            }
        });
        let result = parse_track_response(&json, "1Z12345E0291980793").expect("parse failed");
        assert_eq!(result.tracking_number, "1Z12345E0291980793");
        assert_eq!(result.carrier_code, "ups");
        assert_eq!(result.status, "Delivered");
        assert_eq!(result.location, Some("Beverly Hills, CA".to_string()));
        assert_eq!(result.estimated_delivery, Some("20260408".to_string()));
        assert_eq!(result.events.len(), 2);
        assert_eq!(result.events[0].description, "Delivered");
        assert_eq!(result.events[1].description, "Out For Delivery Today");
    }

    #[test]
    fn parse_track_response_handles_empty_activities() {
        let json = serde_json::json!({
            "trackResponse": {
                "shipment": [{
                    "package": [{ "activity": [] }]
                }]
            }
        });
        let result = parse_track_response(&json, "1Z000").expect("parse failed");
        assert_eq!(result.status, "UNKNOWN");
        assert!(result.events.is_empty());
    }

    #[test]
    fn parse_track_response_errors_on_missing_shipment() {
        let json = serde_json::json!({ "trackResponse": {} });
        assert!(parse_track_response(&json, "1Z000").is_err());
    }
}
