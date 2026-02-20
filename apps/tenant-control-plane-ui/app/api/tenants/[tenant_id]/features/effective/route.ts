// ============================================================
// GET /api/tenants/[tenant_id]/features/effective
// BFF aggregation endpoint — merges plan entitlements, bundle
// entitlements, and tenant-specific overrides into one list
// with explicit source attribution.
// Auth: requires platform_admin JWT in httpOnly cookie
// Falls back to seed data when TTP is unavailable.
// ============================================================
import { NextRequest, NextResponse } from 'next/server';
import { guardPlatformAdmin } from '@/lib/server/auth';
import { TTP_BASE_URL } from '@/lib/constants';
import { EffectiveEntitlementListResponseSchema } from '@/lib/api/types';
import type { EffectiveEntitlement, EffectiveEntitlementListResponse } from '@/lib/api/types';

// Seed data for when TTP is unavailable — demonstrates all three source types
const SEED_EFFECTIVE: EffectiveEntitlement[] = [
  {
    code: 'max_users',
    name: 'Maximum Users',
    granted: 25,
    source: 'plan',
    source_name: 'Professional',
  },
  {
    code: 'api_access',
    name: 'API Access',
    granted: true,
    source: 'plan',
    source_name: 'Professional',
  },
  {
    code: 'storage_gb',
    name: 'Storage (GB)',
    granted: 50,
    source: 'bundle',
    source_name: 'Analytics Add-on',
  },
  {
    code: 'sso_enabled',
    name: 'Single Sign-On',
    granted: true,
    source: 'bundle',
    source_name: 'Compliance Suite',
  },
  {
    code: 'custom_domain',
    name: 'Custom Domain',
    granted: true,
    source: 'override',
    justification: 'Approved by sales for enterprise pilot',
  },
  {
    code: 'support_tier',
    name: 'Support Tier',
    granted: 'premium',
    source: 'override',
    justification: 'Escalated per contract addendum',
  },
  {
    code: 'audit_log_retention_days',
    name: 'Audit Log Retention (Days)',
    granted: 365,
    source: 'plan',
    source_name: 'Professional',
  },
  {
    code: 'advanced_analytics',
    name: 'Advanced Analytics',
    granted: true,
    source: 'bundle',
    source_name: 'Analytics Add-on',
  },
];

export async function GET(
  _request: NextRequest,
  { params }: { params: Promise<{ tenant_id: string }> }
) {
  const auth = await guardPlatformAdmin();
  if (auth instanceof Response) return auth;

  const { tenant_id } = await params;

  try {
    const upstreamUrl = `${TTP_BASE_URL}/api/ttp/tenants/${encodeURIComponent(tenant_id)}/features/effective`;
    const res = await fetch(upstreamUrl, {
      headers: { 'Content-Type': 'application/json' },
      signal: AbortSignal.timeout(5000),
    });

    if (!res.ok) {
      if (res.status === 404) {
        return NextResponse.json({ entitlements: [], total: 0 });
      }
      return NextResponse.json(
        { error: `Upstream error: ${res.status}` },
        { status: res.status >= 500 ? 502 : res.status },
      );
    }

    const raw = await res.json();
    const parsed = EffectiveEntitlementListResponseSchema.safeParse(raw);
    if (parsed.success) {
      return NextResponse.json(parsed.data);
    }
    return NextResponse.json(raw);
  } catch {
    // TTP unavailable — return seed data so Features tab still renders
    const fallback: EffectiveEntitlementListResponse = {
      entitlements: SEED_EFFECTIVE,
      total: SEED_EFFECTIVE.length,
    };
    return NextResponse.json(fallback);
  }
}
