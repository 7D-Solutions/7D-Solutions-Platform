use crate::*;
use platform_sdk::{parse_response, ClientError, PlatformClient, VerifiedClaims};

pub struct CustomerComplaintsClient {
    client: PlatformClient,
}

impl CustomerComplaintsClient {
    pub fn new(client: PlatformClient) -> Self {
        Self { client }
    }

    // ── Complaints ────────────────────────────────────────────────────────────

    pub async fn list_complaints(
        &self,
        claims: &VerifiedClaims,
    ) -> Result<Vec<Complaint>, ClientError> {
        let resp = self
            .client
            .get("/api/customer-complaints/complaints", claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn get_complaint(
        &self,
        claims: &VerifiedClaims,
        id: uuid::Uuid,
    ) -> Result<ComplaintDetail, ClientError> {
        let path = format!("/api/customer-complaints/complaints/{id}");
        let resp = self
            .client
            .get(&path, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn create_complaint(
        &self,
        claims: &VerifiedClaims,
        body: &CreateComplaintRequest,
    ) -> Result<Complaint, ClientError> {
        let resp = self
            .client
            .post("/api/customer-complaints/complaints", body, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn update_complaint(
        &self,
        claims: &VerifiedClaims,
        id: uuid::Uuid,
        body: &UpdateComplaintRequest,
    ) -> Result<Complaint, ClientError> {
        let path = format!("/api/customer-complaints/complaints/{id}");
        let resp = self
            .client
            .put(&path, body, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn triage_complaint(
        &self,
        claims: &VerifiedClaims,
        id: uuid::Uuid,
        body: &TriageComplaintRequest,
    ) -> Result<Complaint, ClientError> {
        let path = format!("/api/customer-complaints/complaints/{id}/triage");
        let resp = self
            .client
            .post(&path, body, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn start_investigation(
        &self,
        claims: &VerifiedClaims,
        id: uuid::Uuid,
        body: &StartInvestigationRequest,
    ) -> Result<Complaint, ClientError> {
        let path = format!("/api/customer-complaints/complaints/{id}/start-investigation");
        let resp = self
            .client
            .post(&path, body, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn respond_complaint(
        &self,
        claims: &VerifiedClaims,
        id: uuid::Uuid,
        body: &RespondComplaintRequest,
    ) -> Result<Complaint, ClientError> {
        let path = format!("/api/customer-complaints/complaints/{id}/respond");
        let resp = self
            .client
            .post(&path, body, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn close_complaint(
        &self,
        claims: &VerifiedClaims,
        id: uuid::Uuid,
        body: &CloseComplaintRequest,
    ) -> Result<Complaint, ClientError> {
        let path = format!("/api/customer-complaints/complaints/{id}/close");
        let resp = self
            .client
            .post(&path, body, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn cancel_complaint(
        &self,
        claims: &VerifiedClaims,
        id: uuid::Uuid,
        body: &CancelComplaintRequest,
    ) -> Result<Complaint, ClientError> {
        let path = format!("/api/customer-complaints/complaints/{id}/cancel");
        let resp = self
            .client
            .post(&path, body, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn assign_complaint(
        &self,
        claims: &VerifiedClaims,
        id: uuid::Uuid,
        body: &AssignComplaintRequest,
    ) -> Result<Complaint, ClientError> {
        let path = format!("/api/customer-complaints/complaints/{id}/assign");
        let resp = self
            .client
            .post(&path, body, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    // ── Activity & Resolution ─────────────────────────────────────────────────

    pub async fn add_note(
        &self,
        claims: &VerifiedClaims,
        complaint_id: uuid::Uuid,
        body: &CreateActivityLogRequest,
    ) -> Result<ComplaintActivityLog, ClientError> {
        let path = format!("/api/customer-complaints/complaints/{complaint_id}/notes");
        let resp = self
            .client
            .post(&path, body, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn add_customer_communication(
        &self,
        claims: &VerifiedClaims,
        complaint_id: uuid::Uuid,
        body: &CreateActivityLogRequest,
    ) -> Result<ComplaintActivityLog, ClientError> {
        let path =
            format!("/api/customer-complaints/complaints/{complaint_id}/customer-communication");
        let resp = self
            .client
            .post(&path, body, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn list_activity_log(
        &self,
        claims: &VerifiedClaims,
        complaint_id: uuid::Uuid,
    ) -> Result<Vec<ComplaintActivityLog>, ClientError> {
        let path = format!("/api/customer-complaints/complaints/{complaint_id}/activity-log");
        let resp = self
            .client
            .get(&path, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn create_resolution(
        &self,
        claims: &VerifiedClaims,
        complaint_id: uuid::Uuid,
        body: &CreateResolutionRequest,
    ) -> Result<ComplaintResolution, ClientError> {
        let path = format!("/api/customer-complaints/complaints/{complaint_id}/resolution");
        let resp = self
            .client
            .post(&path, body, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn get_resolution(
        &self,
        claims: &VerifiedClaims,
        complaint_id: uuid::Uuid,
    ) -> Result<ComplaintResolution, ClientError> {
        let path = format!("/api/customer-complaints/complaints/{complaint_id}/resolution");
        let resp = self
            .client
            .get(&path, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    // ── Categories ────────────────────────────────────────────────────────────

    pub async fn list_categories(
        &self,
        claims: &VerifiedClaims,
        include_inactive: bool,
    ) -> Result<Vec<ComplaintCategoryCode>, ClientError> {
        let path =
            format!("/api/customer-complaints/categories?include_inactive={include_inactive}");
        let resp = self
            .client
            .get(&path, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn create_category(
        &self,
        claims: &VerifiedClaims,
        body: &CreateCategoryCodeRequest,
    ) -> Result<ComplaintCategoryCode, ClientError> {
        let resp = self
            .client
            .post("/api/customer-complaints/categories", body, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn update_category(
        &self,
        claims: &VerifiedClaims,
        code: &str,
        body: &UpdateCategoryCodeRequest,
    ) -> Result<ComplaintCategoryCode, ClientError> {
        let path = format!("/api/customer-complaints/categories/{code}");
        let resp = self
            .client
            .put(&path, body, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    // ── Labels ────────────────────────────────────────────────────────────────

    pub async fn list_status_labels(
        &self,
        claims: &VerifiedClaims,
    ) -> Result<Vec<CcStatusLabel>, ClientError> {
        let resp = self
            .client
            .get("/api/customer-complaints/status-labels", claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn set_status_label(
        &self,
        claims: &VerifiedClaims,
        canonical: &str,
        body: &UpsertLabelRequest,
    ) -> Result<CcStatusLabel, ClientError> {
        let path = format!("/api/customer-complaints/status-labels/{canonical}");
        let resp = self
            .client
            .put(&path, body, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn list_severity_labels(
        &self,
        claims: &VerifiedClaims,
    ) -> Result<Vec<CcSeverityLabel>, ClientError> {
        let resp = self
            .client
            .get("/api/customer-complaints/severity-labels", claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn set_severity_label(
        &self,
        claims: &VerifiedClaims,
        canonical: &str,
        body: &UpsertLabelRequest,
    ) -> Result<CcSeverityLabel, ClientError> {
        let path = format!("/api/customer-complaints/severity-labels/{canonical}");
        let resp = self
            .client
            .put(&path, body, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn list_source_labels(
        &self,
        claims: &VerifiedClaims,
    ) -> Result<Vec<CcSourceLabel>, ClientError> {
        let resp = self
            .client
            .get("/api/customer-complaints/source-labels", claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn set_source_label(
        &self,
        claims: &VerifiedClaims,
        canonical: &str,
        body: &UpsertLabelRequest,
    ) -> Result<CcSourceLabel, ClientError> {
        let path = format!("/api/customer-complaints/source-labels/{canonical}");
        let resp = self
            .client
            .put(&path, body, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }
}
