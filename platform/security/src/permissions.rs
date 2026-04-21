//! Permission string constants for all platform modules.
//!
//! These are the strings that must appear in a caller's JWT `perms` claim
//! for the request to be authorised.  Constants are grouped by module.
//!
//! Naming convention: `<module>.<action>` where action is one of:
//! - `mutate` — any state-changing operation (POST / PUT / DELETE)
//! - `post`   — financial journal-posting (GL convention)
//! - `read`   — query-only (reserved; not yet enforced by default)

// ── Accounts Receivable ───────────────────────────────────────────────────────

pub const AR_MUTATE: &str = "ar.mutate";
pub const AR_READ: &str = "ar.read";

// ── Payments ──────────────────────────────────────────────────────────────────

pub const PAYMENTS_MUTATE: &str = "payments.mutate";
pub const PAYMENTS_READ: &str = "payments.read";

// ── Subscriptions ─────────────────────────────────────────────────────────────

pub const SUBSCRIPTIONS_MUTATE: &str = "subscriptions.mutate";

// ── General Ledger ────────────────────────────────────────────────────────────

/// Financial journal-posting permission (standard GL term).
pub const GL_POST: &str = "gl.post";
pub const GL_READ: &str = "gl.read";

// ── Notifications ─────────────────────────────────────────────────────────────

pub const NOTIFICATIONS_MUTATE: &str = "notifications.mutate";
pub const NOTIFICATIONS_READ: &str = "notifications.read";

// ── Maintenance ───────────────────────────────────────────────────────────────

pub const MAINTENANCE_MUTATE: &str = "maintenance.mutate";
pub const MAINTENANCE_READ: &str = "maintenance.read";

// ── Inventory ─────────────────────────────────────────────────────────────────

pub const INVENTORY_MUTATE: &str = "inventory.mutate";
pub const INVENTORY_READ: &str = "inventory.read";

// ── Reporting / Analytics ─────────────────────────────────────────────────────

pub const REPORTING_MUTATE: &str = "reporting.mutate";
pub const REPORTING_READ: &str = "reporting.read";

// ── Treasury / Cash Management ────────────────────────────────────────────────

pub const TREASURY_MUTATE: &str = "treasury.mutate";
pub const TREASURY_READ: &str = "treasury.read";

// ── Accounts Payable ─────────────────────────────────────────────────────────

pub const AP_MUTATE: &str = "ap.mutate";
pub const AP_READ: &str = "ap.read";
pub const AP_QUALIFY_VENDOR: &str = "ap.qualify_vendor";

// ── Consolidation ─────────────────────────────────────────────────────────────

pub const CONSOLIDATION_MUTATE: &str = "consolidation.mutate";
pub const CONSOLIDATION_READ: &str = "consolidation.read";

// ── Timekeeping ───────────────────────────────────────────────────────────────

pub const TIMEKEEPING_MUTATE: &str = "timekeeping.mutate";
pub const TIMEKEEPING_READ: &str = "timekeeping.read";

// ── Fixed Assets ─────────────────────────────────────────────────────────────

pub const FIXED_ASSETS_MUTATE: &str = "fixed_assets.mutate";
pub const FIXED_ASSETS_READ: &str = "fixed_assets.read";

// ── Party ────────────────────────────────────────────────────────────────────

pub const PARTY_MUTATE: &str = "party.mutate";
pub const PARTY_READ: &str = "party.read";

// ── Integrations ─────────────────────────────────────────────────────────────

pub const INTEGRATIONS_MUTATE: &str = "integrations.mutate";
pub const INTEGRATIONS_READ: &str = "integrations.read";

// OAuth admin — dev-only token import. Requires env gate (OAUTH_IMPORT_ENABLED=1
// or non-production ENV) in addition to this permission.
pub const INTEGRATIONS_OAUTH_ADMIN: &str = "integrations.oauth.admin";

// Sync sub-capabilities — each endpoint requires its own explicit permission.
// Never accept the coarse INTEGRATIONS_MUTATE for these; authority flip in
// particular touches financially sensitive sync-ownership state.
pub const INTEGRATIONS_SYNC_AUTHORITY_FLIP: &str = "integrations.sync.authority.flip";
pub const INTEGRATIONS_SYNC_CONFLICT_RESOLVE: &str = "integrations.sync.conflict.resolve";
pub const INTEGRATIONS_SYNC_PUSH: &str = "integrations.sync.push";
pub const INTEGRATIONS_SYNC_READ: &str = "integrations.sync.read";

// ── TTP (Third-Party Processing) ─────────────────────────────────────────────

pub const TTP_MUTATE: &str = "ttp.mutate";
pub const TTP_READ: &str = "ttp.read";

// ── PDF Editor ───────────────────────────────────────────────────────────────

pub const PDF_EDITOR_MUTATE: &str = "pdf_editor.mutate";
pub const PDF_EDITOR_READ: &str = "pdf_editor.read";

// ── TrashTech Pro (vertical product) ─────────────────────────────────────────

pub const TRASHTECH_MUTATE: &str = "trashtech.mutate";
pub const TRASHTECH_READ: &str = "trashtech.read";

// ── Shipping / Receiving ────────────────────────────────────────────────

pub const SHIPPING_RECEIVING_MUTATE: &str = "shipping_receiving.mutate";
pub const SHIPPING_RECEIVING_READ: &str = "shipping_receiving.read";

// ── Document Management ─────────────────────────────────────────────────

pub const DOC_MGMT_MUTATE: &str = "doc_mgmt.mutate";
pub const DOC_MGMT_READ: &str = "doc_mgmt.read";

// ── Workflow ───────────────────────────────────────────────────────────

pub const WORKFLOW_MUTATE: &str = "workflow.mutate";
pub const WORKFLOW_READ: &str = "workflow.read";

// ── Numbering ──────────────────────────────────────────────────────────

pub const NUMBERING_ALLOCATE: &str = "numbering.allocate";
pub const NUMBERING_READ: &str = "numbering.read";

// ── BOM (Bill of Materials) ──────────────────────────────────────────

pub const BOM_MUTATE: &str = "bom.mutate";
pub const BOM_READ: &str = "bom.read";

// ── Workforce Competence ─────────────────────────────────────────────

pub const WORKFORCE_COMPETENCE_MUTATE: &str = "workforce_competence.mutate";
pub const WORKFORCE_COMPETENCE_READ: &str = "workforce_competence.read";

// ── Quality Inspection ──────────────────────────────────────────────

pub const QUALITY_INSPECTION_MUTATE: &str = "quality_inspection.mutate";
pub const QUALITY_INSPECTION_READ: &str = "quality_inspection.read";

// ── Production ──────────────────────────────────────────────────

pub const PRODUCTION_MUTATE: &str = "production.mutate";
pub const PRODUCTION_READ: &str = "production.read";
pub const PRODUCTION_TIME_ENTRY_APPROVE: &str = "production.time_entry.approve";

// ── Outside Processing ──────────────────────────────────────────

pub const OUTSIDE_PROCESSING_MUTATE: &str = "outside_processing.mutate";
pub const OUTSIDE_PROCESSING_READ: &str = "outside_processing.read";

// ── Sales Orders ────────────────────────────────────────────────

pub const SALES_ORDERS_MUTATE: &str = "sales_orders.mutate";
pub const SALES_ORDERS_READ: &str = "sales_orders.read";

// ── CRM Pipeline ────────────────────────────────────────────────

pub const CRM_PIPELINE_MUTATE: &str = "crm_pipeline.mutate";
pub const CRM_PIPELINE_READ: &str = "crm_pipeline.read";

// ── Customer Complaints ──────────────────────────────────────────────────────

pub const CUSTOMER_COMPLAINTS_MUTATE: &str = "customer_complaints.mutate";
pub const CUSTOMER_COMPLAINTS_READ: &str = "customer_complaints.read";
pub const CC_COMPLAINT_TRIAGE: &str = "customer_complaints.complaint.triage";
pub const CC_COMPLAINT_CLOSE: &str = "customer_complaints.complaint.close";
pub const CC_COMPLAINT_CANCEL: &str = "customer_complaints.complaint.cancel";
pub const CC_CATEGORY_MANAGE: &str = "customer_complaints.category.manage";
pub const CC_LABELS_EDIT: &str = "customer_complaints.labels.edit";
pub const CC_ADMIN_SWEEP: &str = "customer_complaints.admin.sweep";

// ── Shop Floor Gates ─────────────────────────────────────────────────────────

pub const SHOP_FLOOR_GATES_MUTATE: &str = "shop_floor_gates.mutate";
pub const SHOP_FLOOR_GATES_READ: &str = "shop_floor_gates.read";

// ── Customer Portal ─────────────────────────────────────────────

/// Administer customer portal users/docs — distinct from party record management.
pub const CUSTOMER_PORTAL_ADMIN: &str = "customer_portal.admin";

// ── Platform Control Plane ───────────────────────────────────────────────────

/// Create a new tenant and trigger provisioning.
/// Required on POST /api/control/tenants.
pub const PLATFORM_TENANTS_CREATE: &str = "platform.tenants.create";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_permissions_ar_constants_are_non_empty() {
        assert!(!AR_MUTATE.is_empty());
        assert!(!AR_READ.is_empty());
    }

    #[test]
    fn test_permissions_payments_constants_are_non_empty() {
        assert!(!PAYMENTS_MUTATE.is_empty());
        assert!(!PAYMENTS_READ.is_empty());
    }

    #[test]
    fn test_permissions_gl_constants_are_non_empty() {
        assert!(!GL_POST.is_empty());
        assert!(!GL_READ.is_empty());
    }

    #[test]
    fn test_permissions_inventory_constants_are_non_empty() {
        assert!(!INVENTORY_MUTATE.is_empty());
        assert!(!INVENTORY_READ.is_empty());
    }

    #[test]
    fn test_permissions_treasury_constants_are_non_empty() {
        assert!(!TREASURY_MUTATE.is_empty());
        assert!(!TREASURY_READ.is_empty());
    }

    #[test]
    fn test_permissions_ap_constants_are_non_empty() {
        assert!(!AP_MUTATE.is_empty());
        assert!(!AP_READ.is_empty());
    }

    #[test]
    fn test_permissions_consolidation_constants_are_non_empty() {
        assert!(!CONSOLIDATION_MUTATE.is_empty());
        assert!(!CONSOLIDATION_READ.is_empty());
    }

    #[test]
    fn test_permissions_timekeeping_constants_are_non_empty() {
        assert!(!TIMEKEEPING_MUTATE.is_empty());
        assert!(!TIMEKEEPING_READ.is_empty());
    }

    #[test]
    fn test_permissions_fixed_assets_constants_are_non_empty() {
        assert!(!FIXED_ASSETS_MUTATE.is_empty());
        assert!(!FIXED_ASSETS_READ.is_empty());
    }

    #[test]
    fn test_permissions_party_constants_are_non_empty() {
        assert!(!PARTY_MUTATE.is_empty());
        assert!(!PARTY_READ.is_empty());
    }

    #[test]
    fn test_permissions_integrations_constants_are_non_empty() {
        assert!(!INTEGRATIONS_MUTATE.is_empty());
        assert!(!INTEGRATIONS_READ.is_empty());
    }

    #[test]
    fn test_permissions_integrations_sync_constants_are_non_empty() {
        assert!(!INTEGRATIONS_SYNC_AUTHORITY_FLIP.is_empty());
        assert!(!INTEGRATIONS_SYNC_CONFLICT_RESOLVE.is_empty());
        assert!(!INTEGRATIONS_SYNC_PUSH.is_empty());
        assert!(!INTEGRATIONS_SYNC_READ.is_empty());
    }

    #[test]
    fn test_permissions_integrations_sync_distinct_from_coarse() {
        // Sync caps must NOT equal the coarse mutate/read permissions —
        // guards must reject a broad token for sync endpoints.
        assert_ne!(INTEGRATIONS_SYNC_AUTHORITY_FLIP, INTEGRATIONS_MUTATE);
        assert_ne!(INTEGRATIONS_SYNC_CONFLICT_RESOLVE, INTEGRATIONS_MUTATE);
        assert_ne!(INTEGRATIONS_SYNC_PUSH, INTEGRATIONS_MUTATE);
        assert_ne!(INTEGRATIONS_SYNC_READ, INTEGRATIONS_READ);
    }

    #[test]
    fn test_permissions_integrations_sync_all_distinct() {
        let caps = [
            INTEGRATIONS_SYNC_AUTHORITY_FLIP,
            INTEGRATIONS_SYNC_CONFLICT_RESOLVE,
            INTEGRATIONS_SYNC_PUSH,
            INTEGRATIONS_SYNC_READ,
        ];
        for i in 0..caps.len() {
            for j in (i + 1)..caps.len() {
                assert_ne!(caps[i], caps[j], "sync permissions must all be distinct");
            }
        }
    }

    #[test]
    fn test_permissions_integrations_sync_follow_dot_convention() {
        for perm in &[
            INTEGRATIONS_SYNC_AUTHORITY_FLIP,
            INTEGRATIONS_SYNC_CONFLICT_RESOLVE,
            INTEGRATIONS_SYNC_PUSH,
            INTEGRATIONS_SYNC_READ,
        ] {
            assert!(perm.contains('.'), "sync permission '{}' must contain a dot", perm);
            assert!(
                perm.starts_with("integrations.sync."),
                "sync permission '{}' must start with 'integrations.sync.'",
                perm
            );
        }
    }

    #[test]
    fn test_permissions_ttp_constants_are_non_empty() {
        assert!(!TTP_MUTATE.is_empty());
        assert!(!TTP_READ.is_empty());
    }

    #[test]
    fn test_permissions_pdf_editor_constants_are_non_empty() {
        assert!(!PDF_EDITOR_MUTATE.is_empty());
        assert!(!PDF_EDITOR_READ.is_empty());
    }

    #[test]
    fn test_permissions_notifications_constants_are_non_empty() {
        assert!(!NOTIFICATIONS_MUTATE.is_empty());
        assert!(!NOTIFICATIONS_READ.is_empty());
    }

    #[test]
    fn test_permissions_notifications_mutate_distinct_from_read() {
        assert_ne!(NOTIFICATIONS_MUTATE, NOTIFICATIONS_READ);
    }

    #[test]
    fn test_permissions_bom_constants_are_non_empty() {
        assert!(!BOM_MUTATE.is_empty());
        assert!(!BOM_READ.is_empty());
    }

    #[test]
    fn test_permissions_follow_dot_convention() {
        // Every mutate permission must follow "module.mutate" or "module.post" pattern
        let mutate_perms = [
            AR_MUTATE,
            PAYMENTS_MUTATE,
            SUBSCRIPTIONS_MUTATE,
            GL_POST,
            NOTIFICATIONS_MUTATE,
            MAINTENANCE_MUTATE,
            INVENTORY_MUTATE,
            REPORTING_MUTATE,
            TREASURY_MUTATE,
            AP_MUTATE,
            CONSOLIDATION_MUTATE,
            TIMEKEEPING_MUTATE,
            FIXED_ASSETS_MUTATE,
            PARTY_MUTATE,
            INTEGRATIONS_MUTATE,
            TTP_MUTATE,
            PDF_EDITOR_MUTATE,
            SHIPPING_RECEIVING_MUTATE,
            WORKFLOW_MUTATE,
            WORKFORCE_COMPETENCE_MUTATE,
            BOM_MUTATE,
            QUALITY_INSPECTION_MUTATE,
            PRODUCTION_MUTATE,
            PRODUCTION_TIME_ENTRY_APPROVE,
            OUTSIDE_PROCESSING_MUTATE,
            SALES_ORDERS_MUTATE,
            CRM_PIPELINE_MUTATE,
            PLATFORM_TENANTS_CREATE,
        ];
        for perm in &mutate_perms {
            assert!(
                perm.contains('.'),
                "Permission '{}' must contain a dot",
                perm
            );
            let parts: Vec<&str> = perm.splitn(2, '.').collect();
            assert_eq!(
                parts.len(),
                2,
                "Permission '{}' must have exactly one dot",
                perm
            );
            assert!(
                !parts[0].is_empty(),
                "Module prefix in '{}' must not be empty",
                perm
            );
            assert!(
                !parts[1].is_empty(),
                "Action in '{}' must not be empty",
                perm
            );
        }
    }

    #[test]
    fn test_permissions_mutate_distinct_from_read() {
        assert_ne!(AR_MUTATE, AR_READ);
        assert_ne!(PAYMENTS_MUTATE, PAYMENTS_READ);
        assert_ne!(GL_POST, GL_READ);
        assert_ne!(MAINTENANCE_MUTATE, MAINTENANCE_READ);
        assert_ne!(INVENTORY_MUTATE, INVENTORY_READ);
        assert_ne!(TREASURY_MUTATE, TREASURY_READ);
        assert_ne!(AP_MUTATE, AP_READ);
        assert_ne!(CONSOLIDATION_MUTATE, CONSOLIDATION_READ);
        assert_ne!(TIMEKEEPING_MUTATE, TIMEKEEPING_READ);
        assert_ne!(FIXED_ASSETS_MUTATE, FIXED_ASSETS_READ);
        assert_ne!(PARTY_MUTATE, PARTY_READ);
        assert_ne!(INTEGRATIONS_MUTATE, INTEGRATIONS_READ);
        assert_ne!(TTP_MUTATE, TTP_READ);
        assert_ne!(PDF_EDITOR_MUTATE, PDF_EDITOR_READ);
        assert_ne!(SHIPPING_RECEIVING_MUTATE, SHIPPING_RECEIVING_READ);
        assert_ne!(WORKFLOW_MUTATE, WORKFLOW_READ);
        assert_ne!(WORKFORCE_COMPETENCE_MUTATE, WORKFORCE_COMPETENCE_READ);
        assert_ne!(REPORTING_MUTATE, REPORTING_READ);
        assert_ne!(BOM_MUTATE, BOM_READ);
        assert_ne!(QUALITY_INSPECTION_MUTATE, QUALITY_INSPECTION_READ);
        assert_ne!(PRODUCTION_MUTATE, PRODUCTION_READ);
        assert_ne!(SALES_ORDERS_MUTATE, SALES_ORDERS_READ);
    }

    #[test]
    fn test_permissions_sales_orders_constants_are_non_empty() {
        assert!(!SALES_ORDERS_MUTATE.is_empty());
        assert!(!SALES_ORDERS_READ.is_empty());
    }

    #[test]
    fn test_permissions_production_constants_are_non_empty() {
        assert!(!PRODUCTION_MUTATE.is_empty());
        assert!(!PRODUCTION_READ.is_empty());
        assert!(!PRODUCTION_TIME_ENTRY_APPROVE.is_empty());
    }

    #[test]
    fn test_permissions_reporting_constants_are_non_empty() {
        assert!(!REPORTING_MUTATE.is_empty());
        assert!(!REPORTING_READ.is_empty());
    }

    #[test]
    fn test_permissions_customer_portal_admin_is_non_empty() {
        assert!(!CUSTOMER_PORTAL_ADMIN.is_empty());
        assert!(CUSTOMER_PORTAL_ADMIN.contains('.'));
    }

    #[test]
    fn test_permissions_shipping_receiving_constants_are_non_empty() {
        assert!(!SHIPPING_RECEIVING_MUTATE.is_empty());
        assert!(!SHIPPING_RECEIVING_READ.is_empty());
    }

    #[test]
    fn test_permissions_workflow_constants_are_non_empty() {
        assert!(!WORKFLOW_MUTATE.is_empty());
        assert!(!WORKFLOW_READ.is_empty());
    }

    #[test]
    fn test_permissions_crm_pipeline_constants_are_non_empty() {
        assert!(!CRM_PIPELINE_MUTATE.is_empty());
        assert!(!CRM_PIPELINE_READ.is_empty());
    }

    #[test]
    fn test_permissions_crm_pipeline_mutate_distinct_from_read() {
        assert_ne!(CRM_PIPELINE_MUTATE, CRM_PIPELINE_READ);
    }

    #[test]
    fn test_permissions_customer_complaints_constants_are_non_empty() {
        assert!(!CUSTOMER_COMPLAINTS_MUTATE.is_empty());
        assert!(!CUSTOMER_COMPLAINTS_READ.is_empty());
    }

    #[test]
    fn test_permissions_customer_complaints_mutate_distinct_from_read() {
        assert_ne!(CUSTOMER_COMPLAINTS_MUTATE, CUSTOMER_COMPLAINTS_READ);
    }

    #[test]
    fn test_permissions_cc_fine_grained_are_non_empty() {
        assert!(!CC_COMPLAINT_TRIAGE.is_empty());
        assert!(!CC_COMPLAINT_CLOSE.is_empty());
        assert!(!CC_COMPLAINT_CANCEL.is_empty());
        assert!(!CC_CATEGORY_MANAGE.is_empty());
        assert!(!CC_LABELS_EDIT.is_empty());
        assert!(!CC_ADMIN_SWEEP.is_empty());
    }

    #[test]
    fn test_permissions_cc_fine_grained_contain_dot() {
        for perm in &[CC_COMPLAINT_TRIAGE, CC_COMPLAINT_CLOSE, CC_COMPLAINT_CANCEL,
                      CC_CATEGORY_MANAGE, CC_LABELS_EDIT, CC_ADMIN_SWEEP] {
            assert!(perm.contains('.'), "Permission '{}' must contain a dot", perm);
        }
    }
}
