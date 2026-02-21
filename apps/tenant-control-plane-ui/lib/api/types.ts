// ============================================================
// Tenant API types — shared between BFF routes and client code
// ============================================================
import { z } from 'zod';

// ── Tenant Summary (list item) ──────────────────────────────

export const TenantSummarySchema = z.object({
  id: z.string(),
  name: z.string(),
  status: z.string(),
  plan: z.string(),
  app_id: z.string().optional(),
  created_at: z.string().optional(),
  updated_at: z.string().optional(),
});

export type TenantSummary = z.infer<typeof TenantSummarySchema>;

// ── Tenant List Response (paginated) ────────────────────────

export const TenantListResponseSchema = z.object({
  tenants: z.array(TenantSummarySchema),
  total: z.number(),
  page: z.number(),
  page_size: z.number(),
});

export type TenantListResponse = z.infer<typeof TenantListResponseSchema>;

// ── Tenant Filters (query params) ───────────────────────────

export type TenantFilter = {
  [key: string]: string;
  search: string;
  status: string;
  plan: string;
  app_id: string;
};

export const DEFAULT_TENANT_FILTERS: TenantFilter = {
  search: '',
  status: '',
  plan: '',
  app_id: '',
};

// ── Filter option sets ──────────────────────────────────────

export const TENANT_STATUS_OPTIONS = [
  { value: '', label: 'All statuses' },
  { value: 'active', label: 'Active' },
  { value: 'suspended', label: 'Suspended' },
  { value: 'pending', label: 'Setting up' },
  { value: 'terminated', label: 'Terminated' },
  { value: 'trial', label: 'Trial' },
] as const;

export const TENANT_PLAN_OPTIONS = [
  { value: '', label: 'All plans' },
  { value: 'Starter', label: 'Starter' },
  { value: 'Professional', label: 'Professional' },
  { value: 'Enterprise', label: 'Enterprise' },
] as const;

// ── Plan Summary (catalog list item) ────────────────────────

export const PlanSummarySchema = z.object({
  id: z.string(),
  name: z.string(),
  pricing_model: z.string(),
  included_seats: z.number(),
  metered_dimensions: z.array(z.string()),
  status: z.string(),
  created_at: z.string().optional(),
});

export type PlanSummary = z.infer<typeof PlanSummarySchema>;

// ── Plan List Response (paginated) ──────────────────────────

export const PlanListResponseSchema = z.object({
  plans: z.array(PlanSummarySchema),
  total: z.number(),
  page: z.number(),
  page_size: z.number(),
});

export type PlanListResponse = z.infer<typeof PlanListResponseSchema>;

// ── Plan Detail (single plan with full associations) ────────

export const PricingRuleSchema = z.object({
  id: z.string(),
  label: z.string(),
  type: z.string(),
  amount: z.number(),
  currency: z.string().optional(),
  per_unit: z.string().optional(),
  tier_min: z.number().optional(),
  tier_max: z.number().optional(),
});

export type PricingRule = z.infer<typeof PricingRuleSchema>;

export const MeteredDimensionDetailSchema = z.object({
  key: z.string(),
  label: z.string(),
  unit: z.string(),
  included_quota: z.number().optional(),
  overage_rate: z.number().optional(),
});

export type MeteredDimensionDetail = z.infer<typeof MeteredDimensionDetailSchema>;

export const BundleRefSchema = z.object({
  id: z.string(),
  name: z.string(),
  status: z.string(),
});

export type BundleRef = z.infer<typeof BundleRefSchema>;

export const EntitlementRefSchema = z.object({
  id: z.string(),
  key: z.string(),
  label: z.string(),
  value_type: z.string(),
  value: z.union([z.string(), z.number(), z.boolean()]),
});

export type EntitlementRef = z.infer<typeof EntitlementRefSchema>;

export const PlanDetailSchema = z.object({
  id: z.string(),
  name: z.string(),
  description: z.string().optional(),
  pricing_model: z.string(),
  included_seats: z.number(),
  status: z.string(),
  created_at: z.string().optional(),
  updated_at: z.string().optional(),
  pricing_rules: z.array(PricingRuleSchema),
  metered_dimensions: z.array(MeteredDimensionDetailSchema),
  bundles: z.array(BundleRefSchema),
  entitlements: z.array(EntitlementRefSchema),
});

export type PlanDetail = z.infer<typeof PlanDetailSchema>;

// ── Plan Status Options ─────────────────────────────────────

export const PLAN_STATUS_OPTIONS = [
  { value: '', label: 'All statuses' },
  { value: 'active', label: 'Active' },
  { value: 'draft', label: 'Draft' },
  { value: 'archived', label: 'Archived' },
] as const;

// ── Notification (backend-persisted) ─────────────────────────

export const NotificationSeveritySchema = z.enum(['info', 'success', 'warning', 'error']);

export const NotificationSchema = z.object({
  id: z.string(),
  severity: NotificationSeveritySchema,
  title: z.string(),
  message: z.string().optional(),
  timestamp: z.string(),
  read: z.boolean(),
});

export type Notification = z.infer<typeof NotificationSchema>;

export const NotificationListResponseSchema = z.object({
  notifications: z.array(NotificationSchema),
  unread_count: z.number(),
});

export type NotificationListResponse = z.infer<typeof NotificationListResponseSchema>;

export const MarkReadRequestSchema = z.object({
  ids: z.array(z.string()).optional(),
  all: z.boolean().optional(),
});

export type MarkReadRequest = z.infer<typeof MarkReadRequestSchema>;

// ── Bundle Summary (list item — lightweight, no composition) ─

export const BundleSummarySchema = z.object({
  id: z.string(),
  name: z.string(),
  status: z.string(),
  entitlement_count: z.number(),
  created_at: z.string().optional(),
});

export type BundleSummary = z.infer<typeof BundleSummarySchema>;

// ── Bundle List Response (paginated) ─────────────────────────

export const BundleListResponseSchema = z.object({
  bundles: z.array(BundleSummarySchema),
  total: z.number(),
  page: z.number(),
  page_size: z.number(),
});

export type BundleListResponse = z.infer<typeof BundleListResponseSchema>;

// ── Bundle Detail (full composition) ─────────────────────────

export const BundleDetailSchema = z.object({
  id: z.string(),
  name: z.string(),
  status: z.string(),
  description: z.string().optional(),
  entitlements: z.array(EntitlementRefSchema),
  created_at: z.string().optional(),
  updated_at: z.string().optional(),
});

export type BundleDetail = z.infer<typeof BundleDetailSchema>;

// ── Bundle Status Options ────────────────────────────────────

export const BUNDLE_STATUS_OPTIONS = [
  { value: '', label: 'All statuses' },
  { value: 'active', label: 'Active' },
  { value: 'draft', label: 'Draft' },
  { value: 'archived', label: 'Archived' },
] as const;

// ── Tenant Detail (single tenant) ──────────────────────────

export const TenantDetailSchema = z.object({
  id: z.string(),
  name: z.string(),
  status: z.string(),
  plan: z.string(),
  app_id: z.string().optional(),
  created_at: z.string().optional(),
  updated_at: z.string().optional(),
  activated_at: z.string().optional(),
  suspended_at: z.string().optional(),
  terminated_at: z.string().optional(),
  user_count: z.number().optional(),
  seat_limit: z.number().optional(),
});

export type TenantDetail = z.infer<typeof TenantDetailSchema>;

// ── Tenant Plan Summary ─────────────────────────────────────

export const TenantPlanSummarySchema = z.object({
  plan_id: z.string(),
  plan_name: z.string(),
  pricing_model: z.string(),
  included_seats: z.number(),
  metered_dimensions: z.array(z.string()),
  assigned_at: z.string().optional(),
});

export type TenantPlanSummary = z.infer<typeof TenantPlanSummarySchema>;

// ── Plan Assignment Request ─────────────────────────────────

export const PlanAssignmentRequestSchema = z.object({
  plan_id: z.string().min(1, 'Plan is required'),
  effective_date: z.string().min(1, 'Effective date is required').refine(
    (val) => /^\d{4}-\d{2}-\d{2}$/.test(val),
    { message: 'Must be a valid date (YYYY-MM-DD)' },
  ).refine(
    (val) => {
      // Compare date strings directly to avoid UTC vs local timezone issues.
      // Both todayString and <input type="date"> produce YYYY-MM-DD in local time.
      const today = new Date();
      const yyyy = today.getFullYear();
      const mm = String(today.getMonth() + 1).padStart(2, '0');
      const dd = String(today.getDate()).padStart(2, '0');
      return val >= `${yyyy}-${mm}-${dd}`;
    },
    { message: 'Effective date cannot be in the past' },
  ),
});

export type PlanAssignmentRequest = z.infer<typeof PlanAssignmentRequestSchema>;

// ── Entitlement Summary (catalog list item) ─────────────────

export const EntitlementSummarySchema = z.object({
  id: z.string(),
  key: z.string(),
  label: z.string(),
  value_type: z.string(),
  default_value: z.union([z.string(), z.number(), z.boolean()]),
  status: z.string(),
  created_at: z.string().optional(),
});

export type EntitlementSummary = z.infer<typeof EntitlementSummarySchema>;

// ── Entitlement List Response (paginated) ────────────────────

export const EntitlementListResponseSchema = z.object({
  entitlements: z.array(EntitlementSummarySchema),
  total: z.number(),
  page: z.number(),
  page_size: z.number(),
});

export type EntitlementListResponse = z.infer<typeof EntitlementListResponseSchema>;

// ── Entitlement Value Type Options ───────────────────────────

export const ENTITLEMENT_VALUE_TYPE_OPTIONS = [
  { value: '', label: 'All types' },
  { value: 'boolean', label: 'Boolean' },
  { value: 'number', label: 'Number' },
  { value: 'string', label: 'String' },
] as const;

// ── Entitlement Status Options ───────────────────────────────

export const ENTITLEMENT_STATUS_OPTIONS = [
  { value: '', label: 'All statuses' },
  { value: 'active', label: 'Active' },
  { value: 'draft', label: 'Draft' },
  { value: 'archived', label: 'Archived' },
] as const;

// ── Audit Event Summary (list item) ─────────────────────────

export const AuditEventSummarySchema = z.object({
  id: z.string(),
  timestamp: z.string(),
  actor: z.string(),
  action: z.string(),
  tenant_id: z.string().optional(),
  tenant_name: z.string().optional(),
  resource_type: z.string().optional(),
  resource_id: z.string().optional(),
  summary: z.string().optional(),
  payload: z.unknown().optional(),
});

export type AuditEventSummary = z.infer<typeof AuditEventSummarySchema>;

// ── Audit List Response (paginated) ─────────────────────────

export const AuditListResponseSchema = z.object({
  events: z.array(AuditEventSummarySchema),
  total: z.number(),
  page: z.number(),
  page_size: z.number(),
});

export type AuditListResponse = z.infer<typeof AuditListResponseSchema>;

// ── Audit Filter defaults ───────────────────────────────────

export type AuditFilter = {
  [key: string]: string;
  actor: string;
  action: string;
  tenant_id: string;
  date_from: string;
  date_to: string;
};

export const DEFAULT_AUDIT_FILTERS: AuditFilter = {
  actor: '',
  action: '',
  tenant_id: '',
  date_from: '',
  date_to: '',
};

export const AUDIT_ACTION_OPTIONS = [
  { value: '', label: 'All actions' },
  { value: 'tenant.created', label: 'Tenant Created' },
  { value: 'tenant.updated', label: 'Tenant Updated' },
  { value: 'tenant.suspended', label: 'Tenant Suspended' },
  { value: 'tenant.activated', label: 'Tenant Activated' },
  { value: 'plan.created', label: 'Plan Created' },
  { value: 'plan.updated', label: 'Plan Updated' },
  { value: 'user.login', label: 'User Login' },
  { value: 'user.logout', label: 'User Logout' },
  { value: 'settings.changed', label: 'Settings Changed' },
] as const;

// ── Tenant User (Access tab) ─────────────────────────────────

export const TenantUserSchema = z.object({
  id: z.string(),
  email: z.string(),
  name: z.string().optional(),
  status: z.string(),
  last_seen: z.string().optional(),
  created_at: z.string().optional(),
});

export type TenantUser = z.infer<typeof TenantUserSchema>;

export const TenantUserListResponseSchema = z.object({
  users: z.array(TenantUserSchema),
  total: z.number(),
});

export type TenantUserListResponse = z.infer<typeof TenantUserListResponseSchema>;

// ── Effective Entitlement (tenant-scoped, with attribution) ──

export const EffectiveEntitlementSchema = z.object({
  code: z.string(),
  name: z.string(),
  granted: z.union([z.string(), z.number(), z.boolean()]),
  source: z.enum(['plan', 'bundle', 'override']),
  source_name: z.string().optional(),
  justification: z.string().optional(),
});

export type EffectiveEntitlement = z.infer<typeof EffectiveEntitlementSchema>;

export const EffectiveEntitlementListResponseSchema = z.object({
  entitlements: z.array(EffectiveEntitlementSchema),
  total: z.number(),
});

export type EffectiveEntitlementListResponse = z.infer<typeof EffectiveEntitlementListResponseSchema>;

// ── Feature Override Request (grant/revoke) ─────────────────

export const FeatureOverrideRequestSchema = z.object({
  entitlement_code: z.string().min(1, 'Entitlement code is required'),
  action: z.enum(['grant', 'revoke']),
  justification: z.string().min(1, 'Justification is required').max(500, 'Justification must be 500 characters or fewer'),
});

export type FeatureOverrideRequest = z.infer<typeof FeatureOverrideRequestSchema>;

// ── Invoice Summary (tenant-scoped list item) ────────────────

export const InvoiceSummarySchema = z.object({
  id: z.string(),
  number: z.string().optional(),
  status: z.string(),
  total: z.number().optional(),
  currency: z.string().optional(),
  issued_at: z.string().optional(),
  due_date: z.string().optional(),
  paid_at: z.string().optional(),
});

export type InvoiceSummary = z.infer<typeof InvoiceSummarySchema>;

// ── Invoice List Response (paginated) ────────────────────────

export const InvoiceListResponseSchema = z.object({
  invoices: z.array(InvoiceSummarySchema),
  total: z.number(),
  page: z.number(),
  page_size: z.number(),
});

export type InvoiceListResponse = z.infer<typeof InvoiceListResponseSchema>;

// ── Invoice Line Item ────────────────────────────────────────

export const InvoiceLineItemSchema = z.object({
  id: z.string(),
  description: z.string(),
  quantity: z.number(),
  unit_price: z.number(),
  amount: z.number(),
  currency: z.string().optional(),
});

export type InvoiceLineItem = z.infer<typeof InvoiceLineItemSchema>;

// ── Invoice Detail (single invoice with line items) ──────────

export const InvoiceDetailSchema = z.object({
  id: z.string(),
  tenant_id: z.string(),
  number: z.string().optional(),
  status: z.string(),
  total: z.number().optional(),
  subtotal: z.number().optional(),
  tax: z.number().optional(),
  currency: z.string().optional(),
  issued_at: z.string().optional(),
  due_date: z.string().optional(),
  paid_at: z.string().optional(),
  line_items: z.array(InvoiceLineItemSchema),
});

export type InvoiceDetail = z.infer<typeof InvoiceDetailSchema>;

// ── Invoice Filter defaults ──────────────────────────────────

export type InvoiceFilter = {
  [key: string]: string;
  status: string;
  date_from: string;
  date_to: string;
};

export const DEFAULT_INVOICE_FILTERS: InvoiceFilter = {
  status: '',
  date_from: '',
  date_to: '',
};

export const INVOICE_STATUS_OPTIONS = [
  { value: '', label: 'All statuses' },
  { value: 'draft', label: 'Draft' },
  { value: 'issued', label: 'Issued' },
  { value: 'paid', label: 'Paid' },
  { value: 'overdue', label: 'Overdue' },
  { value: 'void', label: 'Void' },
] as const;

// ── Admin Tool Requests ─────────────────────────────────────

export const RunBillingRequestSchema = z.object({
  tenant_id: z.string().optional().or(z.literal('')),
  billing_period: z
    .string()
    .regex(/^\d{4}-\d{2}$/, 'Must be in YYYY-MM format')
    .optional()
    .or(z.literal('')),
  reason: z.string().min(1, 'Reason is required').max(500, 'Reason must be 500 characters or fewer'),
});

export type RunBillingRequest = z.infer<typeof RunBillingRequestSchema>;

export const ReconcileMappingRequestSchema = z.object({
  tenant_id: z.string().min(1, 'Tenant ID is required'),
  reason: z.string().min(1, 'Reason is required').max(500, 'Reason must be 500 characters or fewer'),
});

export type ReconcileMappingRequest = z.infer<typeof ReconcileMappingRequestSchema>;

export interface AdminToolResult {
  ok: boolean;
  message?: string;
  not_available?: boolean;
}

// ── Health Snapshot (service readiness) ─────────────────────

export const ServiceHealthSchema = z.object({
  service: z.string(),
  status: z.enum(['available', 'degraded', 'unavailable']),
  latency_ms: z.number().optional(),
});

export type ServiceHealth = z.infer<typeof ServiceHealthSchema>;

export const HealthSnapshotSchema = z.object({
  services: z.array(ServiceHealthSchema),
  checked_at: z.string(),
});

export type HealthSnapshot = z.infer<typeof HealthSnapshotSchema>;

// ── Billing Overview (aggregated BFF DTO) ────────────────────

export const SectionAvailabilitySchema = z.enum(['available', 'unavailable', 'error']);
export type SectionAvailability = z.infer<typeof SectionAvailabilitySchema>;

export const BillingChargesSchema = z.object({
  availability: SectionAvailabilitySchema,
  base_amount: z.number().optional(),
  seat_count: z.number().optional(),
  seat_unit_price: z.number().optional(),
  seat_total: z.number().optional(),
  metered_charges: z.array(z.object({
    dimension: z.string(),
    quantity: z.number(),
    amount: z.number(),
  })).optional(),
  total: z.number().optional(),
  currency: z.string().optional(),
});

export const BillingLastInvoiceSchema = z.object({
  availability: SectionAvailabilitySchema,
  id: z.string().optional(),
  number: z.string().optional(),
  issued_at: z.string().optional(),
  due_date: z.string().optional(),
  total: z.number().optional(),
  status: z.string().optional(),
  currency: z.string().optional(),
});

export const BillingOutstandingSchema = z.object({
  availability: SectionAvailabilitySchema,
  total_due: z.number().optional(),
  overdue_count: z.number().optional(),
  currency: z.string().optional(),
});

export const BillingPaymentStatusSchema = z.object({
  availability: SectionAvailabilitySchema,
  status: z.string().optional(),
  last_payment_at: z.string().optional(),
  last_payment_amount: z.number().optional(),
  currency: z.string().optional(),
});

export const BillingDunningSchema = z.object({
  availability: SectionAvailabilitySchema,
  state: z.string().optional(),
  current_step: z.number().optional(),
  total_steps: z.number().optional(),
  next_retry_at: z.string().optional(),
  started_at: z.string().optional(),
});

export const BillingOverviewSchema = z.object({
  charges: BillingChargesSchema,
  last_invoice: BillingLastInvoiceSchema,
  outstanding: BillingOutstandingSchema,
  payment_status: BillingPaymentStatusSchema,
  dunning: BillingDunningSchema,
});

export type BillingOverview = z.infer<typeof BillingOverviewSchema>;

// ── Support Session ─────────────────────────────────────────

export const StartSupportSessionRequestSchema = z.object({
  reason: z.string().min(1, 'Reason is required').max(500, 'Reason must be 500 characters or fewer'),
});

export type StartSupportSessionRequest = z.infer<typeof StartSupportSessionRequestSchema>;

// ── RBAC Role (tenant-scoped) ─────────────────────────────

export const RbacRoleSchema = z.object({
  id: z.string(),
  name: z.string(),
  description: z.string().optional(),
  permissions: z.array(z.string()),
});

export type RbacRole = z.infer<typeof RbacRoleSchema>;

// ── RBAC User Grant (user → roles mapping) ────────────────

export const RbacUserGrantSchema = z.object({
  user_id: z.string(),
  email: z.string(),
  name: z.string().optional(),
  roles: z.array(z.string()),
});

export type RbacUserGrant = z.infer<typeof RbacUserGrantSchema>;

// ── RBAC Snapshot (aggregated view) ───────────────────────

export const RbacSnapshotResponseSchema = z.object({
  roles: z.array(RbacRoleSchema),
  user_roles: z.array(RbacUserGrantSchema),
});

export type RbacSnapshotResponse = z.infer<typeof RbacSnapshotResponseSchema>;

// ── RBAC Grant/Revoke Request ─────────────────────────────

export const RbacChangeRequestSchema = z.object({
  user_id: z.string().min(1, 'User is required'),
  role_id: z.string().min(1, 'Role is required'),
  action: z.enum(['grant', 'revoke']),
});

export type RbacChangeRequest = z.infer<typeof RbacChangeRequestSchema>;

// ── Tenant Subscribed Apps (App Launcher) ────────────────

export const TenantAppSchema = z.object({
  id: z.string(),
  name: z.string(),
  module_code: z.string(),
  launch_url: z.string().nullable(),
  status: z.enum(['available', 'provisioning', 'unavailable']),
});

export type TenantApp = z.infer<typeof TenantAppSchema>;

export const TenantAppListResponseSchema = z.object({
  apps: z.array(TenantAppSchema),
});

export type TenantAppListResponse = z.infer<typeof TenantAppListResponseSchema>;

// ── Create Tenant Request ─────────────────────────────────

export const CreateTenantRequestSchema = z.object({
  name: z.string().min(1, 'Name is required').max(100, 'Name must be 100 characters or fewer'),
  plan: z.string().min(1, 'Plan is required'),
  environment: z.enum(['development', 'staging', 'production'], {
    errorMap: () => ({ message: 'Environment is required' }),
  }),
});

export type CreateTenantRequest = z.infer<typeof CreateTenantRequestSchema>;
