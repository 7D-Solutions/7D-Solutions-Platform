//! USPS Web Tools carrier provider.
//!
//! Implements `CarrierProvider` for USPS via the legacy XML Web Tools API:
//! - Rate quotes:      RateV4 API
//! - Label creation:   eVS Label API
//! - Tracking:         TrackV2 API
//!
//! ## Config JSON
//! ```json
//! {
//!   "user_id":  "XXXX1234567",
//!   "base_url": "https://stg-production.shippingapis.com/ShippingAPI.dll"
//! }
//! ```
//! `base_url` is optional — defaults to the USPS production endpoint.
//!
//! ## Invariant
//! Provider struct is zero-state. `config` is read on every call so credentials
//! can rotate without restarting the process.

use async_trait::async_trait;
use quick_xml::events::Event;
use quick_xml::Reader;
use reqwest::Client;
use serde_json::Value;

use super::{
    CarrierProvider, CarrierProviderError, LabelPdfResponse, LabelResult, RateQuote, TrackingEvent,
    TrackingResult,
};

const USPS_PRODUCTION_URL: &str = "https://production.shippingapis.com/ShippingAPI.dll";

pub struct UspsCarrierProvider;

// ── Credential + config helpers ───────────────────────────────

fn get_user_id(config: &Value) -> Result<&str, CarrierProviderError> {
    config["user_id"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            CarrierProviderError::CredentialsError(
                "USPS config missing required field 'user_id'".to_string(),
            )
        })
}

fn get_base_url(config: &Value) -> &str {
    config["base_url"]
        .as_str()
        .filter(|s| !s.is_empty())
        .unwrap_or(USPS_PRODUCTION_URL)
}

// ── XML request builders ──────────────────────────────────────

fn rate_v4_xml(user_id: &str, req: &Value) -> String {
    let origin = req["origin_zip"].as_str().unwrap_or("10001");
    let dest = req["dest_zip"].as_str().unwrap_or("90210");
    let pounds = req["weight_lbs"].as_u64().unwrap_or(1);
    let ounces = req["weight_oz"].as_u64().unwrap_or(0);
    let length = req["length_in"].as_u64().unwrap_or(12);
    let width = req["width_in"].as_u64().unwrap_or(12);
    let height = req["height_in"].as_u64().unwrap_or(12);

    format!(
        concat!(
            r#"<RateV4Request USERID="{uid}"><Revision>2</Revision>"#,
            r#"<Package ID="0"><Service>ALL</Service>"#,
            r#"<ZipOrigination>{orig}</ZipOrigination>"#,
            r#"<ZipDestination>{dest}</ZipDestination>"#,
            r#"<Pounds>{lbs}</Pounds><Ounces>{oz}</Ounces>"#,
            r#"<Container></Container>"#,
            r#"<Width>{w}</Width><Length>{l}</Length><Height>{h}</Height>"#,
            r#"<Girth></Girth><Machinable>TRUE</Machinable>"#,
            r#"</Package></RateV4Request>"#,
        ),
        uid = user_id,
        orig = origin,
        dest = dest,
        lbs = pounds,
        oz = ounces,
        w = width,
        l = length,
        h = height,
    )
}

fn evs_xml(user_id: &str, req: &Value) -> String {
    let from_name = req["from_name"].as_str().unwrap_or("Sender");
    let from_addr = req["from_address"].as_str().unwrap_or("123 Main St");
    let from_city = req["from_city"].as_str().unwrap_or("New York");
    let from_state = req["from_state"].as_str().unwrap_or("NY");
    let from_zip = req["from_zip"].as_str().unwrap_or("10001");
    let to_name = req["to_name"].as_str().unwrap_or("Recipient");
    let to_addr = req["to_address"].as_str().unwrap_or("456 Sunset Blvd");
    let to_city = req["to_city"].as_str().unwrap_or("Beverly Hills");
    let to_state = req["to_state"].as_str().unwrap_or("CA");
    let to_zip = req["to_zip"].as_str().unwrap_or("90210");
    // Weight in ounces: prefer explicit weight_oz, else convert weight_lbs
    let weight_oz = req["weight_oz"]
        .as_u64()
        .or_else(|| req["weight_lbs"].as_u64().map(|lbs| lbs * 16))
        .unwrap_or(160);
    let length = req["length_in"].as_u64().unwrap_or(12);
    let width = req["width_in"].as_u64().unwrap_or(12);
    let height = req["height_in"].as_u64().unwrap_or(12);

    format!(
        concat!(
            r#"<eVSRequest USERID="{uid}"><Option/><Revision>1</Revision>"#,
            r#"<ImageParameters><ImageType>PDF</ImageType></ImageParameters>"#,
            r#"<FromName>{fn_}</FromName><FromFirm/><FromAddress1/>"#,
            r#"<FromAddress2>{fa}</FromAddress2>"#,
            r#"<FromCity>{fc}</FromCity><FromState>{fs}</FromState>"#,
            r#"<FromZip5>{fz}</FromZip5><FromZip4/>"#,
            r#"<ToName>{tn}</ToName><ToFirm/><ToAddress1/>"#,
            r#"<ToAddress2>{ta}</ToAddress2>"#,
            r#"<ToCity>{tc}</ToCity><ToState>{ts}</ToState>"#,
            r#"<ToZip5>{tz}</ToZip5><ToZip4/>"#,
            r#"<WeightInOunces>{woz}</WeightInOunces>"#,
            r#"<ServiceType>PRIORITY</ServiceType>"#,
            r#"<WaiverOfSignature/>"#,
            r#"<Length>{l}</Length><Width>{w}</Width><Height>{h}</Height><Girth/>"#,
            r#"<LabelDate/><CustomerRefNo/><AddressServiceRequested/>"#,
            r#"<ExpressMailOptions/><ShipDate/><InsuredAmount/>"#,
            r#"<HazMatContent/><RFIDSerialNumber/>"#,
            r#"<LinearHandlingCode/><BarcodeNumber/><SpecialServices/>"#,
            r#"</eVSRequest>"#,
        ),
        uid = user_id,
        fn_ = from_name,
        fa = from_addr,
        fc = from_city,
        fs = from_state,
        fz = from_zip,
        tn = to_name,
        ta = to_addr,
        tc = to_city,
        ts = to_state,
        tz = to_zip,
        woz = weight_oz,
        l = length,
        w = width,
        h = height,
    )
}

fn track_v2_xml(user_id: &str, tracking_number: &str) -> String {
    format!(
        r#"<TrackRequest USERID="{uid}"><TrackID ID="{tn}"/></TrackRequest>"#,
        uid = user_id,
        tn = tracking_number,
    )
}

// ── HTTP dispatch ─────────────────────────────────────────────

async fn usps_get(base_url: &str, api: &str, xml: &str) -> Result<String, CarrierProviderError> {
    let client = Client::new();
    let resp = client
        .get(base_url)
        .query(&[("API", api), ("XML", xml)])
        .send()
        .await
        .map_err(|e| CarrierProviderError::ApiError(format!("USPS HTTP error: {e}")))?;

    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| CarrierProviderError::ApiError(format!("USPS response read error: {e}")))?;

    if !status.is_success() {
        return Err(CarrierProviderError::ApiError(format!(
            "USPS HTTP {status}: {body}"
        )));
    }

    Ok(body)
}

// ── XML parsing helpers ───────────────────────────────────────

/// Check whether the USPS response XML contains a top-level `<Error>` and, if
/// so, extract the description and return a `CarrierProviderError::ApiError`.
fn check_for_usps_error(xml: &str) -> Result<(), CarrierProviderError> {
    if !xml.contains("<Error>") {
        return Ok(());
    }
    let desc =
        xml_first_text(xml, "Description").unwrap_or_else(|| "Unknown USPS API error".to_string());
    let number = xml_first_text(xml, "Number").unwrap_or_default();
    Err(CarrierProviderError::ApiError(format!(
        "USPS error {number}: {desc}"
    )))
}

/// Extract the text content of the first occurrence of `<tag>…</tag>` using
/// quick-xml's pull parser. Returns `None` if the tag is absent or empty.
fn xml_first_text(xml: &str, tag: &str) -> Option<String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let tag_bytes = tag.as_bytes();
    let mut buf = Vec::new();
    let mut in_tag = false;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) if e.name().as_ref() == tag_bytes => {
                in_tag = true;
            }
            Ok(Event::Text(ref e)) if in_tag => {
                let text = e.unescape().ok()?.into_owned();
                let cleaned = strip_html_tags(&text);
                if cleaned.is_empty() {
                    return None;
                }
                return Some(cleaned);
            }
            Ok(Event::End(ref e)) if e.name().as_ref() == tag_bytes => {
                in_tag = false;
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    None
}

/// Strip inline HTML tags (e.g. `<sup>&#8482;</sup>`) that USPS embeds in
/// service names.
fn strip_html_tags(s: &str) -> String {
    // Decode HTML entities FIRST so that entity-encoded tags like
    // &lt;sup&gt;&#8482;&lt;/sup&gt; become <sup>™</sup> before stripping.
    let decoded = s
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#8482;", "\u{2122}");

    let mut out = String::with_capacity(decoded.len());
    let mut in_tag = false;
    for ch in decoded.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out.trim().to_string()
}

// ── RateV4 response parser ────────────────────────────────────

fn parse_rate_response(xml: &str) -> Result<Vec<RateQuote>, CarrierProviderError> {
    check_for_usps_error(xml)?;

    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();

    let mut quotes: Vec<RateQuote> = Vec::new();
    // State for the current <Postage> block
    let mut in_postage = false;
    let mut current_tag = String::new();
    let mut service = String::new();
    let mut rate_str = String::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => {
                let qname = e.name();
                let name = std::str::from_utf8(qname.as_ref()).unwrap_or("");
                if name == "Postage" {
                    in_postage = true;
                    service.clear();
                    rate_str.clear();
                } else if in_postage {
                    current_tag = name.to_string();
                }
            }
            Ok(Event::Text(ref e)) if in_postage => {
                let text = e
                    .unescape()
                    .map_err(|err| CarrierProviderError::ApiError(format!("XML decode: {err}")))?
                    .into_owned();
                match current_tag.as_str() {
                    "MailService" => service = strip_html_tags(&text),
                    "Rate" => rate_str = text.trim().to_string(),
                    _ => {}
                }
            }
            Ok(Event::End(ref e)) => {
                let qname = e.name();
                let name = std::str::from_utf8(qname.as_ref()).unwrap_or("");
                if name == "Postage" && in_postage {
                    in_postage = false;
                    current_tag.clear();
                    if !service.is_empty() && !rate_str.is_empty() {
                        let rate_cents =
                            (rate_str.parse::<f64>().unwrap_or(0.0) * 100.0).round() as i64;
                        quotes.push(RateQuote {
                            service_level: service.clone(),
                            carrier_code: "usps".to_string(),
                            total_charge_minor: rate_cents,
                            currency: "USD".to_string(),
                            estimated_days: None,
                        });
                    }
                } else if name != "Postage" {
                    current_tag.clear();
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => {
                return Err(CarrierProviderError::ApiError(format!(
                    "USPS RateV4 XML parse error: {e}"
                )));
            }
            _ => {}
        }
        buf.clear();
    }

    if quotes.is_empty() {
        return Err(CarrierProviderError::ApiError(
            "USPS RateV4 returned no rate quotes".to_string(),
        ));
    }

    Ok(quotes)
}

// ── eVS response parser ───────────────────────────────────────

fn parse_evs_response(xml: &str) -> Result<LabelResult, CarrierProviderError> {
    check_for_usps_error(xml)?;

    let tracking = xml_first_text(xml, "BarcodeNumber").ok_or_else(|| {
        CarrierProviderError::ApiError("USPS eVS response missing BarcodeNumber".to_string())
    })?;

    let label_data = xml_first_text(xml, "LabelImage").ok_or_else(|| {
        CarrierProviderError::ApiError("USPS eVS response missing LabelImage".to_string())
    })?;

    Ok(LabelResult {
        tracking_number: tracking,
        label_format: "pdf".to_string(),
        label_data,
        carrier_code: "usps".to_string(),
    })
}

// ── TrackV2 response parser ───────────────────────────────────

/// Parses a `<TrackSummary>` or `<TrackDetail>` XML block into a TrackingEvent.
fn parse_track_block(block: &str) -> Option<TrackingEvent> {
    let event = xml_first_text(block, "Event")?;
    let date = xml_first_text(block, "EventDate").unwrap_or_default();
    let time = xml_first_text(block, "EventTime").unwrap_or_default();
    let city = xml_first_text(block, "EventCity").unwrap_or_default();
    let state = xml_first_text(block, "EventState").unwrap_or_default();

    let timestamp = if date.is_empty() && time.is_empty() {
        String::new()
    } else {
        format!("{date} {time}").trim().to_string()
    };

    let location = if city.is_empty() && state.is_empty() {
        None
    } else {
        Some(
            format!("{city}, {state}")
                .trim_matches(',')
                .trim()
                .to_string(),
        )
    };

    Some(TrackingEvent {
        timestamp,
        description: event,
        location,
    })
}

/// Extract a sub-section of XML between `<tag …>` and `</tag>`.
fn xml_extract_block<'a>(xml: &'a str, tag: &str) -> Option<&'a str> {
    let close_tag = format!("</{tag}>");
    // Handle both `<Tag>` and `<Tag …attr…>` openers
    let open_needle = format!("<{tag}");
    let start = xml.find(&open_needle)?;
    // Advance past the full opening tag (skip to first `>`)
    let inner_start = xml[start..].find('>')? + start + 1;
    let inner_end = xml[inner_start..].find(&close_tag)? + inner_start;
    Some(&xml[inner_start..inner_end])
}

/// Extract ALL sub-section blocks for a repeating tag.
fn xml_extract_all_blocks<'a>(xml: &'a str, tag: &str) -> Vec<&'a str> {
    let close_tag = format!("</{tag}>");
    let open_needle = format!("<{tag}");
    let mut blocks = Vec::new();
    let mut remaining = xml;

    while let Some(start) = remaining.find(&open_needle) {
        let after_open = remaining[start..].find('>').map(|p| start + p + 1);
        let Some(inner_start) = after_open else { break };
        let Some(end_offset) = remaining[inner_start..].find(&close_tag) else {
            break;
        };
        let inner_end = inner_start + end_offset;
        blocks.push(&remaining[inner_start..inner_end]);
        remaining = &remaining[inner_end + close_tag.len()..];
    }

    blocks
}

fn parse_track_response(
    xml: &str,
    tracking_number: &str,
) -> Result<TrackingResult, CarrierProviderError> {
    check_for_usps_error(xml)?;

    let track_info = xml_extract_block(xml, "TrackInfo").ok_or_else(|| {
        CarrierProviderError::ApiError(
            "USPS TrackV2 response missing TrackInfo element".to_string(),
        )
    })?;

    // Summary = current status (first event)
    let summary_block = xml_extract_block(track_info, "TrackSummary");
    let summary_event = summary_block.and_then(parse_track_block);

    let status = summary_event
        .as_ref()
        .map(|e| e.description.clone())
        .unwrap_or_else(|| "UNKNOWN".to_string());

    let location = summary_event.as_ref().and_then(|e| e.location.clone());

    // Collect all events: summary first, then details
    let mut events: Vec<TrackingEvent> = Vec::new();
    if let Some(ev) = summary_event {
        events.push(ev);
    }

    for block in xml_extract_all_blocks(track_info, "TrackDetail") {
        if let Some(ev) = parse_track_block(block) {
            events.push(ev);
        }
    }

    Ok(TrackingResult {
        tracking_number: tracking_number.to_string(),
        carrier_code: "usps".to_string(),
        status,
        location,
        estimated_delivery: None,
        events,
    })
}

// ── CarrierProvider implementation ───────────────────────────

#[async_trait]
impl CarrierProvider for UspsCarrierProvider {
    fn carrier_code(&self) -> &str {
        "usps"
    }

    async fn get_rates(
        &self,
        req: &Value,
        config: &Value,
    ) -> Result<Vec<RateQuote>, CarrierProviderError> {
        let uid = get_user_id(config)?;
        let url = get_base_url(config);
        let xml = rate_v4_xml(uid, req);
        let response = usps_get(url, "RateV4", &xml).await?;
        parse_rate_response(&response)
    }

    async fn create_label(
        &self,
        req: &Value,
        config: &Value,
    ) -> Result<LabelResult, CarrierProviderError> {
        let uid = get_user_id(config)?;
        let url = get_base_url(config);
        let xml = evs_xml(uid, req);
        let response = usps_get(url, "eVS", &xml).await?;
        parse_evs_response(&response)
    }

    async fn create_return_label(
        &self,
        req: &Value,
        config: &Value,
    ) -> Result<LabelResult, CarrierProviderError> {
        // USPS return labels use the eVS Returns endpoint with the same XML
        // structure as eVS but via the "eVSReturnsLabel" API. Addresses are
        // already swapped by the caller (customer=from, warehouse=to).
        let uid = get_user_id(config)?;
        let url = get_base_url(config);
        let xml = evs_xml(uid, req);
        let response = usps_get(url, "eVSReturnsLabel", &xml).await?;
        parse_evs_response(&response)
    }

    async fn track(
        &self,
        tracking_number: &str,
        config: &Value,
    ) -> Result<TrackingResult, CarrierProviderError> {
        let uid = get_user_id(config)?;
        let url = get_base_url(config);
        let xml = track_v2_xml(uid, tracking_number);
        let response = usps_get(url, "TrackV2", &xml).await?;
        parse_track_response(&response, tracking_number)
    }

    async fn fetch_label(
        &self,
        tracking_number: &str,
        config: &Value,
    ) -> Result<LabelPdfResponse, CarrierProviderError> {
        // Requires new USPS REST API credentials in the config:
        //   rest_base_url:    optional, defaults to https://api.usps.com
        //   rest_access_token: OAuth2 bearer token for the USPS REST API
        let rest_base_url = config["rest_base_url"]
            .as_str()
            .filter(|s| !s.is_empty())
            .unwrap_or("https://api.usps.com");
        let token = config["rest_access_token"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                CarrierProviderError::CredentialsError(
                    "USPS config missing 'rest_access_token' for label re-fetch".to_string(),
                )
            })?;

        let client = Client::new();
        let url = format!("{rest_base_url}/labels/v3/{tracking_number}");

        let resp = client
            .get(&url)
            .bearer_auth(token)
            .header("Accept", "application/pdf")
            .send()
            .await
            .map_err(|e| CarrierProviderError::ApiError(format!("USPS REST HTTP error: {e}")))?;

        let status = resp.status();
        if status.as_u16() == 404 {
            return Err(CarrierProviderError::NotFound(format!(
                "USPS: label not found or purged for {tracking_number}"
            )));
        }

        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(CarrierProviderError::ApiError(format!(
                "USPS Labels API error (HTTP {status}): {text}"
            )));
        }

        let pdf_bytes = resp.bytes().await.map_err(|e| {
            CarrierProviderError::ApiError(format!("USPS label response read error: {e}"))
        })?;

        Ok(LabelPdfResponse {
            pdf_bytes: pdf_bytes.to_vec(),
            content_type: "application/pdf".to_string(),
            carrier_reference: tracking_number.to_string(),
        })
    }
}

// ── Unit tests ────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_html_tags_removes_sup_elements() {
        let input = "Priority Mail 2-Day<sup>&#8482;</sup>";
        assert_eq!(strip_html_tags(input), "Priority Mail 2-Day™");
    }

    #[test]
    fn strip_html_tags_no_tags_unchanged() {
        let input = "Priority Mail";
        assert_eq!(strip_html_tags(input), "Priority Mail");
    }

    #[test]
    fn check_for_usps_error_returns_ok_when_clean() {
        let xml = r#"<RateV4Response><Package ID="0"><Postage CLASSID="1"><MailService>Priority Mail</MailService><Rate>16.75</Rate></Postage></Package></RateV4Response>"#;
        assert!(check_for_usps_error(xml).is_ok());
    }

    #[test]
    fn check_for_usps_error_returns_err_on_error_tag() {
        let xml = r#"<Error><Number>80040B19</Number><Description>Authorization failure.</Description></Error>"#;
        let result = check_for_usps_error(xml);
        assert!(result.is_err());
        let msg = format!("{}", result.unwrap_err());
        assert!(msg.contains("Authorization failure"));
    }

    #[test]
    fn parse_rate_response_extracts_all_postage_options() {
        // USPS HTML-escapes the <sup> tags in their XML responses, so they
        // arrive as literal text: &lt;sup&gt;&#8482;&lt;/sup&gt; → <sup>™</sup>
        // which strip_html_tags then removes to leave "Priority Mail 2-Day™".
        let xml = r#"<RateV4Response><Package ID="0">
            <Postage CLASSID="1">
                <MailService>Priority Mail 2-Day&lt;sup&gt;&#8482;&lt;/sup&gt;</MailService>
                <Rate>16.75</Rate>
            </Postage>
            <Postage CLASSID="3">
                <MailService>Priority Mail Express 1-Day&lt;sup&gt;&#8482;&lt;/sup&gt;</MailService>
                <Rate>35.50</Rate>
            </Postage>
        </Package></RateV4Response>"#;

        let quotes = parse_rate_response(xml).expect("parse failed");
        assert_eq!(quotes.len(), 2);
        assert_eq!(quotes[0].service_level, "Priority Mail 2-Day™");
        assert_eq!(quotes[0].total_charge_minor, 1675);
        assert_eq!(quotes[0].currency, "USD");
        assert_eq!(quotes[0].carrier_code, "usps");
        assert_eq!(quotes[1].service_level, "Priority Mail Express 1-Day™");
        assert_eq!(quotes[1].total_charge_minor, 3550);
    }

    #[test]
    fn parse_rate_response_returns_error_on_usps_error() {
        let xml = r#"<Error><Number>-2147219403</Number><Description>Invalid Origination ZIP Code.</Description></Error>"#;
        let result = parse_rate_response(xml);
        assert!(result.is_err());
        let msg = format!("{}", result.unwrap_err());
        assert!(msg.contains("Invalid Origination ZIP Code"));
    }

    #[test]
    fn parse_evs_response_extracts_tracking_and_label() {
        let xml = r#"<eVSResponse>
            <BarcodeNumber>9261292700399997720105</BarcodeNumber>
            <LabelImage>JVBERi0xLjQ=</LabelImage>
            <Postage>15.00</Postage>
        </eVSResponse>"#;

        let result = parse_evs_response(xml).expect("parse failed");
        assert_eq!(result.tracking_number, "9261292700399997720105");
        assert_eq!(result.label_data, "JVBERi0xLjQ=");
        assert_eq!(result.label_format, "pdf");
        assert_eq!(result.carrier_code, "usps");
    }

    #[test]
    fn parse_evs_response_returns_error_on_usps_error() {
        let xml = r#"<Error><Number>-2147219399</Number><Description>Invalid destination address.</Description></Error>"#;
        let result = parse_evs_response(xml);
        assert!(result.is_err());
        let msg = format!("{}", result.unwrap_err());
        assert!(msg.contains("Invalid destination address"));
    }

    #[test]
    fn parse_track_response_extracts_status_and_events() {
        let xml = r#"<TrackResponse>
            <TrackInfo ID="9400110200882774868522">
                <TrackSummary>
                    <EventTime>3:15 pm</EventTime>
                    <EventDate>March 20, 2026</EventDate>
                    <Event>DELIVERED</Event>
                    <EventCity>LOS ANGELES</EventCity>
                    <EventState>CA</EventState>
                    <EventZIPCode>90210</EventZIPCode>
                </TrackSummary>
                <TrackDetail>
                    <EventTime>8:00 am</EventTime>
                    <EventDate>March 20, 2026</EventDate>
                    <Event>OUT FOR DELIVERY</Event>
                    <EventCity>LOS ANGELES</EventCity>
                    <EventState>CA</EventState>
                    <EventZIPCode>90210</EventZIPCode>
                </TrackDetail>
            </TrackInfo>
        </TrackResponse>"#;

        let result = parse_track_response(xml, "9400110200882774868522").expect("parse failed");
        assert_eq!(result.tracking_number, "9400110200882774868522");
        assert_eq!(result.carrier_code, "usps");
        assert_eq!(result.status, "DELIVERED");
        assert_eq!(result.location, Some("LOS ANGELES, CA".to_string()));
        assert_eq!(result.events.len(), 2);
        assert_eq!(result.events[0].description, "DELIVERED");
        assert_eq!(result.events[1].description, "OUT FOR DELIVERY");
    }

    #[test]
    fn parse_track_response_returns_error_on_usps_error() {
        let xml = r#"<TrackResponse>
            <TrackInfo ID="INVALID">
                <Error><Number>-2147219283</Number><Description>The Tracking Number is not associated with a Package.</Description></Error>
            </TrackInfo>
        </TrackResponse>"#;
        // Error is inside TrackInfo — we still detect it via check_for_usps_error
        let result = parse_track_response(xml, "INVALID");
        assert!(result.is_err());
    }

    #[test]
    fn missing_user_id_returns_credentials_error() {
        let config = serde_json::json!({});
        let result = get_user_id(&config);
        assert!(matches!(
            result,
            Err(CarrierProviderError::CredentialsError(_))
        ));
    }

    #[test]
    fn rate_v4_xml_builds_valid_xml_with_defaults() {
        let xml = rate_v4_xml("TESTUSER", &serde_json::json!({}));
        assert!(xml.contains(r#"USERID="TESTUSER""#));
        assert!(xml.contains("<ZipOrigination>10001</ZipOrigination>"));
        assert!(xml.contains("<ZipDestination>90210</ZipDestination>"));
        assert!(xml.contains("<Pounds>1</Pounds>"));
    }

    #[test]
    fn evs_xml_builds_valid_xml_with_req_fields() {
        let req = serde_json::json!({
            "from_name": "Acme Corp",
            "from_zip": "10001",
            "to_name": "Bob Smith",
            "to_zip": "90210",
            "weight_lbs": 10,
        });
        let xml = evs_xml("TESTUSER", &req);
        assert!(xml.contains(r#"USERID="TESTUSER""#));
        assert!(xml.contains("<FromName>Acme Corp</FromName>"));
        assert!(xml.contains("<WeightInOunces>160</WeightInOunces>"));
        assert!(xml.contains("<ToZip5>90210</ToZip5>"));
    }

    #[test]
    fn track_v2_xml_embeds_tracking_number() {
        let xml = track_v2_xml("TESTUSER", "9400110200882774868522");
        assert!(xml.contains(r#"USERID="TESTUSER""#));
        assert!(xml.contains(r#"ID="9400110200882774868522""#));
    }
}
