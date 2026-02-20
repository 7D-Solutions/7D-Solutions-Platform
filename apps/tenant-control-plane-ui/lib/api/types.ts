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
