// ============================================================
// GET /api/tenants/[tenant_id]/plan-summary — BFF proxy to TTP
// Returns the tenant's assigned plan details.
// Falls back to a minimal "Unknown" plan when TTP is unavailable.
// Auth: requires platform_admin JWT in httpOnly cookie
// ============================================================
import { NextRequest, NextResponse } from 'next/server';
import { guardPlatformAdmin } from '@/lib/server/auth';
import { TTP_BASE_URL } from '@/lib/constants';
import { TenantPlanSummarySchema } from '@/lib/api/types';
import type { TenantPlanSummary } from '@/lib/api/types';

export async function GET(
  _request: NextRequest,
  { params }: { params: Promise<{ tenant_id: string }> }
) {
  const auth = await guardPlatformAdmin();
  if (auth instanceof Response) return auth;

  const { tenant_id } = await params;

  try {
    const upstreamUrl = `${TTP_BASE_URL}/api/ttp/tenants/${encodeURIComponent(tenant_id)}/plan`;
    const res = await fetch(upstreamUrl, {
      headers: { 'Content-Type': 'application/json' },
      signal: AbortSignal.timeout(5000),
    });

    if (!res.ok) {
      // If TTP returns 404, the tenant may not have a plan assigned yet
      if (res.status === 404) {
        const noPlan: TenantPlanSummary = {
          plan_id: '',
          plan_name: 'No plan assigned',
          pricing_model: '—',
          included_seats: 0,
          metered_dimensions: [],
        };
        return NextResponse.json(noPlan);
      }
      return NextResponse.json(
        { error: `Upstream error: ${res.status}` },
        { status: res.status >= 500 ? 502 : res.status },
      );
    }

    const raw = await res.json();
    const parsed = TenantPlanSummarySchema.safeParse(raw);
    if (parsed.success) {
      return NextResponse.json(parsed.data);
    }
    return NextResponse.json(raw);
  } catch {
    // TTP unavailable — return fallback so overview still renders
    const fallback: TenantPlanSummary = {
      plan_id: '',
      plan_name: 'Unavailable',
      pricing_model: '—',
      included_seats: 0,
      metered_dimensions: [],
    };
    return NextResponse.json(fallback);
  }
}
