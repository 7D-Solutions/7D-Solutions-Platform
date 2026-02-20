// ============================================================
// GET /api/plans/[plan_id] — BFF aggregation for plan detail
// Returns pricing rules, metered dimensions, bundles, entitlements
// in a single DTO. Falls back to seed data when TTP is unavailable.
// ============================================================
import { NextRequest, NextResponse } from 'next/server';
import { guardPlatformAdmin } from '@/lib/server/auth';
import { TTP_BASE_URL } from '@/lib/constants';
import { PlanDetailSchema } from '@/lib/api/types';
import type { PlanDetail } from '@/lib/api/types';

// Seed plan details returned when TTP is unavailable
const SEED_PLAN_DETAILS: Record<string, PlanDetail> = {
  'plan-starter': {
    id: 'plan-starter',
    name: 'Starter',
    description: 'Entry-level plan for small teams getting started.',
    pricing_model: 'flat',
    included_seats: 5,
    status: 'active',
    created_at: '2026-01-01T00:00:00Z',
    updated_at: '2026-01-15T00:00:00Z',
    pricing_rules: [
      { id: 'pr-s1', label: 'Monthly flat fee', type: 'flat', amount: 49, currency: 'USD' },
    ],
    metered_dimensions: [],
    bundles: [
      { id: 'bun-core', name: 'Core Features', status: 'active' },
    ],
    entitlements: [
      { id: 'ent-s1', key: 'max_projects', label: 'Max Projects', value_type: 'number', value: 10 },
      { id: 'ent-s2', key: 'support_tier', label: 'Support Tier', value_type: 'string', value: 'email' },
    ],
  },
  'plan-professional': {
    id: 'plan-professional',
    name: 'Professional',
    description: 'For growing teams that need usage-based scaling.',
    pricing_model: 'per_seat',
    included_seats: 25,
    status: 'active',
    created_at: '2026-01-01T00:00:00Z',
    updated_at: '2026-02-01T00:00:00Z',
    pricing_rules: [
      { id: 'pr-p1', label: 'Base platform fee', type: 'flat', amount: 199, currency: 'USD' },
      { id: 'pr-p2', label: 'Per additional seat', type: 'per_unit', amount: 12, currency: 'USD', per_unit: 'seat' },
    ],
    metered_dimensions: [
      { key: 'api_calls', label: 'API Calls', unit: 'calls', included_quota: 100000, overage_rate: 0.001 },
      { key: 'storage_gb', label: 'Storage', unit: 'GB', included_quota: 50, overage_rate: 0.10 },
    ],
    bundles: [
      { id: 'bun-core', name: 'Core Features', status: 'active' },
      { id: 'bun-analytics', name: 'Advanced Analytics', status: 'active' },
    ],
    entitlements: [
      { id: 'ent-p1', key: 'max_projects', label: 'Max Projects', value_type: 'number', value: 100 },
      { id: 'ent-p2', key: 'support_tier', label: 'Support Tier', value_type: 'string', value: 'priority' },
      { id: 'ent-p3', key: 'sso_enabled', label: 'SSO Enabled', value_type: 'boolean', value: true },
    ],
  },
  'plan-enterprise': {
    id: 'plan-enterprise',
    name: 'Enterprise',
    description: 'Full-featured plan with tiered pricing and dedicated support.',
    pricing_model: 'tiered',
    included_seats: 100,
    status: 'active',
    created_at: '2026-01-01T00:00:00Z',
    updated_at: '2026-02-10T00:00:00Z',
    pricing_rules: [
      { id: 'pr-e1', label: 'Tier 1 (1-50 seats)', type: 'tiered', amount: 10, currency: 'USD', per_unit: 'seat', tier_min: 1, tier_max: 50 },
      { id: 'pr-e2', label: 'Tier 2 (51-100 seats)', type: 'tiered', amount: 8, currency: 'USD', per_unit: 'seat', tier_min: 51, tier_max: 100 },
      { id: 'pr-e3', label: 'Tier 3 (101+ seats)', type: 'tiered', amount: 6, currency: 'USD', per_unit: 'seat', tier_min: 101 },
    ],
    metered_dimensions: [
      { key: 'api_calls', label: 'API Calls', unit: 'calls', included_quota: 1000000, overage_rate: 0.0005 },
      { key: 'storage_gb', label: 'Storage', unit: 'GB', included_quota: 500, overage_rate: 0.05 },
      { key: 'compute_hours', label: 'Compute Hours', unit: 'hours', included_quota: 1000, overage_rate: 0.02 },
    ],
    bundles: [
      { id: 'bun-core', name: 'Core Features', status: 'active' },
      { id: 'bun-analytics', name: 'Advanced Analytics', status: 'active' },
      { id: 'bun-compliance', name: 'Compliance & Audit', status: 'active' },
    ],
    entitlements: [
      { id: 'ent-e1', key: 'max_projects', label: 'Max Projects', value_type: 'string', value: 'unlimited' },
      { id: 'ent-e2', key: 'support_tier', label: 'Support Tier', value_type: 'string', value: 'dedicated' },
      { id: 'ent-e3', key: 'sso_enabled', label: 'SSO Enabled', value_type: 'boolean', value: true },
      { id: 'ent-e4', key: 'custom_branding', label: 'Custom Branding', value_type: 'boolean', value: true },
    ],
  },
  'plan-trial': {
    id: 'plan-trial',
    name: 'Trial',
    description: 'Free trial with limited features for evaluation.',
    pricing_model: 'flat',
    included_seats: 3,
    status: 'draft',
    created_at: '2026-02-01T00:00:00Z',
    pricing_rules: [
      { id: 'pr-t1', label: 'Trial (no charge)', type: 'flat', amount: 0, currency: 'USD' },
    ],
    metered_dimensions: [],
    bundles: [
      { id: 'bun-core', name: 'Core Features', status: 'active' },
    ],
    entitlements: [
      { id: 'ent-t1', key: 'max_projects', label: 'Max Projects', value_type: 'number', value: 3 },
      { id: 'ent-t2', key: 'support_tier', label: 'Support Tier', value_type: 'string', value: 'community' },
    ],
  },
};

export async function GET(
  _request: NextRequest,
  { params }: { params: Promise<{ plan_id: string }> },
) {
  const auth = await guardPlatformAdmin();
  if (auth instanceof Response) return auth;

  const { plan_id } = await params;

  try {
    const upstreamUrl = `${TTP_BASE_URL}/api/ttp/plans/${encodeURIComponent(plan_id)}`;
    const res = await fetch(upstreamUrl, {
      headers: { 'Content-Type': 'application/json' },
      signal: AbortSignal.timeout(5000),
    });

    if (!res.ok) {
      if (res.status === 404) {
        return NextResponse.json({ error: 'Plan not found' }, { status: 404 });
      }
      return NextResponse.json(
        { error: `Upstream error: ${res.status}` },
        { status: res.status >= 500 ? 502 : res.status },
      );
    }

    const raw = await res.json();
    const parsed = PlanDetailSchema.safeParse(raw);
    if (parsed.success) {
      return NextResponse.json(parsed.data);
    }
    return NextResponse.json(raw);
  } catch {
    // TTP unavailable — return seed plan detail
    const seed = SEED_PLAN_DETAILS[plan_id];
    if (!seed) {
      return NextResponse.json({ error: 'Plan not found' }, { status: 404 });
    }
    return NextResponse.json(seed);
  }
}
