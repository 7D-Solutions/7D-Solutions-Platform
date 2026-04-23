use crate::*;
use platform_sdk::{parse_response, ClientError, PlatformClient, VerifiedClaims};

pub struct CrmPipelineClient {
    client: PlatformClient,
}

impl CrmPipelineClient {
    pub fn new(client: PlatformClient) -> Self {
        Self { client }
    }

    // ── Leads ─────────────────────────────────────────────────────────────────

    pub async fn list_leads(&self, claims: &VerifiedClaims) -> Result<Vec<Lead>, ClientError> {
        let resp = self
            .client
            .get("/api/crm-pipeline/leads", claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn get_lead(
        &self,
        claims: &VerifiedClaims,
        id: uuid::Uuid,
    ) -> Result<Lead, ClientError> {
        let path = format!("/api/crm-pipeline/leads/{id}");
        let resp = self
            .client
            .get(&path, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn create_lead(
        &self,
        claims: &VerifiedClaims,
        body: &CreateLeadRequest,
    ) -> Result<Lead, ClientError> {
        let resp = self
            .client
            .post("/api/crm-pipeline/leads", body, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn update_lead(
        &self,
        claims: &VerifiedClaims,
        id: uuid::Uuid,
        body: &UpdateLeadRequest,
    ) -> Result<Lead, ClientError> {
        let path = format!("/api/crm-pipeline/leads/{id}");
        let resp = self
            .client
            .put(&path, body, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn mark_contacted(
        &self,
        claims: &VerifiedClaims,
        id: uuid::Uuid,
    ) -> Result<Lead, ClientError> {
        let path = format!("/api/crm-pipeline/leads/{id}/contact");
        let resp = self
            .client
            .post(&path, &serde_json::Value::Null, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn mark_qualifying(
        &self,
        claims: &VerifiedClaims,
        id: uuid::Uuid,
    ) -> Result<Lead, ClientError> {
        let path = format!("/api/crm-pipeline/leads/{id}/qualify");
        let resp = self
            .client
            .post(&path, &serde_json::Value::Null, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn mark_qualified(
        &self,
        claims: &VerifiedClaims,
        id: uuid::Uuid,
    ) -> Result<Lead, ClientError> {
        let path = format!("/api/crm-pipeline/leads/{id}/mark-qualified");
        let resp = self
            .client
            .post(&path, &serde_json::Value::Null, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn convert_lead(
        &self,
        claims: &VerifiedClaims,
        id: uuid::Uuid,
        body: &ConvertLeadRequest,
    ) -> Result<ConvertLeadResponse, ClientError> {
        let path = format!("/api/crm-pipeline/leads/{id}/convert");
        let resp = self
            .client
            .post(&path, body, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn disqualify_lead(
        &self,
        claims: &VerifiedClaims,
        id: uuid::Uuid,
        body: &DisqualifyLeadRequest,
    ) -> Result<Lead, ClientError> {
        let path = format!("/api/crm-pipeline/leads/{id}/disqualify");
        let resp = self
            .client
            .post(&path, body, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn mark_dead(
        &self,
        claims: &VerifiedClaims,
        id: uuid::Uuid,
    ) -> Result<Lead, ClientError> {
        let path = format!("/api/crm-pipeline/leads/{id}/mark-dead");
        let resp = self
            .client
            .post(&path, &serde_json::Value::Null, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    // ── Opportunities ─────────────────────────────────────────────────────────

    pub async fn list_opportunities(
        &self,
        claims: &VerifiedClaims,
    ) -> Result<Vec<Opportunity>, ClientError> {
        let resp = self
            .client
            .get("/api/crm-pipeline/opportunities", claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn get_opportunity(
        &self,
        claims: &VerifiedClaims,
        id: uuid::Uuid,
    ) -> Result<Opportunity, ClientError> {
        let path = format!("/api/crm-pipeline/opportunities/{id}");
        let resp = self
            .client
            .get(&path, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn get_opportunity_stage_history(
        &self,
        claims: &VerifiedClaims,
        id: uuid::Uuid,
    ) -> Result<Vec<OpportunityStageHistory>, ClientError> {
        let path = format!("/api/crm-pipeline/opportunities/{id}/stage-history");
        let resp = self
            .client
            .get(&path, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn pipeline_summary(
        &self,
        claims: &VerifiedClaims,
    ) -> Result<Vec<PipelineSummaryItem>, ClientError> {
        let resp = self
            .client
            .get("/api/crm-pipeline/pipeline/summary", claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn create_opportunity(
        &self,
        claims: &VerifiedClaims,
        body: &CreateOpportunityRequest,
    ) -> Result<Opportunity, ClientError> {
        let resp = self
            .client
            .post("/api/crm-pipeline/opportunities", body, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn update_opportunity(
        &self,
        claims: &VerifiedClaims,
        id: uuid::Uuid,
        body: &UpdateOpportunityRequest,
    ) -> Result<Opportunity, ClientError> {
        let path = format!("/api/crm-pipeline/opportunities/{id}");
        let resp = self
            .client
            .put(&path, body, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn advance_stage(
        &self,
        claims: &VerifiedClaims,
        id: uuid::Uuid,
        body: &AdvanceStageRequest,
    ) -> Result<Opportunity, ClientError> {
        let path = format!("/api/crm-pipeline/opportunities/{id}/advance-stage");
        let resp = self
            .client
            .post(&path, body, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn close_won(
        &self,
        claims: &VerifiedClaims,
        id: uuid::Uuid,
        body: &CloseWonRequest,
    ) -> Result<Opportunity, ClientError> {
        let path = format!("/api/crm-pipeline/opportunities/{id}/close-won");
        let resp = self
            .client
            .post(&path, body, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn close_lost(
        &self,
        claims: &VerifiedClaims,
        id: uuid::Uuid,
        body: &CloseLostRequest,
    ) -> Result<Opportunity, ClientError> {
        let path = format!("/api/crm-pipeline/opportunities/{id}/close-lost");
        let resp = self
            .client
            .post(&path, body, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    // ── Pipeline Stages ───────────────────────────────────────────────────────

    pub async fn list_stages(
        &self,
        claims: &VerifiedClaims,
    ) -> Result<Vec<PipelineStage>, ClientError> {
        let resp = self
            .client
            .get("/api/crm-pipeline/stages", claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn create_stage(
        &self,
        claims: &VerifiedClaims,
        body: &CreateStageRequest,
    ) -> Result<PipelineStage, ClientError> {
        let resp = self
            .client
            .post("/api/crm-pipeline/stages", body, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn update_stage(
        &self,
        claims: &VerifiedClaims,
        code: &str,
        body: &UpdateStageRequest,
    ) -> Result<PipelineStage, ClientError> {
        let path = format!("/api/crm-pipeline/stages/{code}");
        let resp = self
            .client
            .put(&path, body, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn deactivate_stage(
        &self,
        claims: &VerifiedClaims,
        code: &str,
    ) -> Result<(), ClientError> {
        let path = format!("/api/crm-pipeline/stages/{code}/deactivate");
        let resp = self
            .client
            .post(&path, &serde_json::Value::Null, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn reorder_stages(
        &self,
        claims: &VerifiedClaims,
        body: &ReorderStagesRequest,
    ) -> Result<Vec<PipelineStage>, ClientError> {
        let resp = self
            .client
            .post("/api/crm-pipeline/stages/reorder", body, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    // ── Activities ────────────────────────────────────────────────────────────

    pub async fn list_activities(
        &self,
        claims: &VerifiedClaims,
    ) -> Result<Vec<Activity>, ClientError> {
        let resp = self
            .client
            .get("/api/crm-pipeline/activities", claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn get_activity(
        &self,
        claims: &VerifiedClaims,
        id: uuid::Uuid,
    ) -> Result<Activity, ClientError> {
        let path = format!("/api/crm-pipeline/activities/{id}");
        let resp = self
            .client
            .get(&path, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn log_activity(
        &self,
        claims: &VerifiedClaims,
        body: &CreateActivityRequest,
    ) -> Result<Activity, ClientError> {
        let resp = self
            .client
            .post("/api/crm-pipeline/activities", body, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn complete_activity(
        &self,
        claims: &VerifiedClaims,
        id: uuid::Uuid,
    ) -> Result<Activity, ClientError> {
        let path = format!("/api/crm-pipeline/activities/{id}/complete");
        let resp = self
            .client
            .post(&path, &serde_json::Value::Null, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn update_activity(
        &self,
        claims: &VerifiedClaims,
        id: uuid::Uuid,
        body: &UpdateActivityRequest,
    ) -> Result<Activity, ClientError> {
        let path = format!("/api/crm-pipeline/activities/{id}");
        let resp = self
            .client
            .put(&path, body, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn list_activity_types(
        &self,
        claims: &VerifiedClaims,
    ) -> Result<Vec<ActivityType>, ClientError> {
        let resp = self
            .client
            .get("/api/crm-pipeline/activity-types", claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn create_activity_type(
        &self,
        claims: &VerifiedClaims,
        body: &CreateActivityTypeRequest,
    ) -> Result<ActivityType, ClientError> {
        let resp = self
            .client
            .post("/api/crm-pipeline/activity-types", body, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn update_activity_type(
        &self,
        claims: &VerifiedClaims,
        code: &str,
        body: &UpdateActivityTypeRequest,
    ) -> Result<ActivityType, ClientError> {
        let path = format!("/api/crm-pipeline/activity-types/{code}");
        let resp = self
            .client
            .put(&path, body, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    // ── Contact Role Attributes ───────────────────────────────────────────────

    pub async fn get_contact_attributes(
        &self,
        claims: &VerifiedClaims,
        party_contact_id: uuid::Uuid,
    ) -> Result<ContactRoleAttributes, ClientError> {
        let path = format!("/api/crm-pipeline/contacts/{party_contact_id}/attributes");
        let resp = self
            .client
            .get(&path, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn set_contact_attributes(
        &self,
        claims: &VerifiedClaims,
        party_contact_id: uuid::Uuid,
        body: &UpsertContactRoleRequest,
    ) -> Result<ContactRoleAttributes, ClientError> {
        let path = format!("/api/crm-pipeline/contacts/{party_contact_id}/attributes");
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
    ) -> Result<Vec<Label>, ClientError> {
        let resp = self
            .client
            .get("/api/crm-pipeline/status-labels", claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn list_source_labels(
        &self,
        claims: &VerifiedClaims,
    ) -> Result<Vec<Label>, ClientError> {
        let resp = self
            .client
            .get("/api/crm-pipeline/source-labels", claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn list_type_labels(
        &self,
        claims: &VerifiedClaims,
    ) -> Result<Vec<Label>, ClientError> {
        let resp = self
            .client
            .get("/api/crm-pipeline/type-labels", claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn list_priority_labels(
        &self,
        claims: &VerifiedClaims,
    ) -> Result<Vec<Label>, ClientError> {
        let resp = self
            .client
            .get("/api/crm-pipeline/priority-labels", claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }
}

impl platform_sdk::PlatformService for CrmPipelineClient {
    const SERVICE_NAME: &'static str = "crm-pipeline";
    fn from_platform_client(client: platform_sdk::PlatformClient) -> Self {
        Self::new(client)
    }
}
