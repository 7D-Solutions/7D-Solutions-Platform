//! Stub carrier provider — returns canned responses for testing.
//!
//! All real carrier adapters (FedEx, UPS, USPS) will follow the same
//! `CarrierProvider` trait. The stub is the only registered implementation
//! until those adapters land.

use async_trait::async_trait;
use serde_json::Value;

use super::{
    CarrierProvider, CarrierProviderError, ChildLabel, LabelResult, MultiPackageLabelRequest,
    MultiPackageLabelResponse, RateQuote, TrackingEvent, TrackingResult,
};

pub struct StubCarrierProvider;

#[async_trait]
impl CarrierProvider for StubCarrierProvider {
    fn carrier_code(&self) -> &str {
        "stub"
    }

    async fn get_rates(
        &self,
        _req: &Value,
        _config: &Value,
    ) -> Result<Vec<RateQuote>, CarrierProviderError> {
        Ok(vec![
            RateQuote {
                service_level: "ground".to_string(),
                carrier_code: "stub".to_string(),
                total_charge_minor: 1500,
                currency: "USD".to_string(),
                estimated_days: Some(5),
            },
            RateQuote {
                service_level: "express".to_string(),
                carrier_code: "stub".to_string(),
                total_charge_minor: 4500,
                currency: "USD".to_string(),
                estimated_days: Some(2),
            },
        ])
    }

    async fn create_label(
        &self,
        _req: &Value,
        _config: &Value,
    ) -> Result<LabelResult, CarrierProviderError> {
        Ok(LabelResult {
            tracking_number: "STUB-TRACK-001".to_string(),
            label_format: "pdf".to_string(),
            label_data: "c3R1Yi1sYWJlbC1kYXRh".to_string(), // base64("stub-label-data")
            carrier_code: "stub".to_string(),
        })
    }

    async fn create_multi_package_label(
        &self,
        req: &MultiPackageLabelRequest,
        _config: &Value,
    ) -> Result<MultiPackageLabelResponse, CarrierProviderError> {
        if req.packages.is_empty() {
            return Err(CarrierProviderError::InvalidRequest(
                "packages must not be empty".to_string(),
            ));
        }
        let children: Vec<ChildLabel> = req
            .packages
            .iter()
            .enumerate()
            .map(|(i, _)| ChildLabel {
                tracking_number: format!("STUB-CHILD-{:03}", i + 1),
                label_url: "c3R1Yi1sYWJlbA==".to_string(),
                package_index: i,
            })
            .collect();
        Ok(MultiPackageLabelResponse {
            master_tracking_number: "STUB-MULTI-MASTER-001".to_string(),
            children,
        })
    }

    async fn create_return_label(
        &self,
        _req: &Value,
        _config: &Value,
    ) -> Result<LabelResult, CarrierProviderError> {
        Ok(LabelResult {
            tracking_number: "STUB-RETURN-001".to_string(),
            label_format: "pdf".to_string(),
            label_data: "c3R1Yi1yZXR1cm4tbGFiZWw=".to_string(), // base64("stub-return-label")
            carrier_code: "stub".to_string(),
        })
    }

    async fn track(
        &self,
        tracking_number: &str,
        _config: &Value,
    ) -> Result<TrackingResult, CarrierProviderError> {
        Ok(TrackingResult {
            tracking_number: tracking_number.to_string(),
            carrier_code: "stub".to_string(),
            status: "in_transit".to_string(),
            location: Some("STUB FACILITY".to_string()),
            estimated_delivery: Some("2026-04-12".to_string()),
            events: vec![TrackingEvent {
                timestamp: "2026-04-08T12:00:00Z".to_string(),
                description: "Package picked up".to_string(),
                location: Some("STUB ORIGIN".to_string()),
            }],
        })
    }
}
