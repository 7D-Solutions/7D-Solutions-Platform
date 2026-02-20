// ============================================================
// GET /api/plans — BFF proxy to TTP plan catalog (cp_plans)
// Forwards query params: status, page, page_size
// Auth: requires platform_admin JWT in httpOnly cookie
// Falls back to seed plan data when TTP is unavailable.
// ============================================================
import { NextRequest, NextResponse } from 'next/server';
import { guardPlatformAdmin } from '@/lib/server/auth';
import { TTP_BASE_URL } from '@/lib/constants';
import { PlanListResponseSchema } from '@/lib/api/types';
import type { PlanListResponse, PlanSummary } from '@/lib/api/types';

// Seed plans returned when TTP plans endpoint is unavailable
const SEED_PLANS: PlanSummary[] = [
  {
    id: 'plan-starter',
    name: 'Starter',
    pricing_model: 'flat',
    included_seats: 5,
    metered_dimensions: [],
    status: 'active',
    created_at: '2026-01-01T00:00:00Z',
  },
  {
    id: 'plan-professional',
    name: 'Professional',
    pricing_model: 'per_seat',
    included_seats: 25,
    metered_dimensions: ['api_calls', 'storage_gb'],
    status: 'active',
    created_at: '2026-01-01T00:00:00Z',
  },
  {
    id: 'plan-enterprise',
    name: 'Enterprise',
    pricing_model: 'tiered',
    included_seats: 100,
    metered_dimensions: ['api_calls', 'storage_gb', 'compute_hours'],
    status: 'active',
    created_at: '2026-01-01T00:00:00Z',
  },
  {
    id: 'plan-trial',
    name: 'Trial',
    pricing_model: 'flat',
    included_seats: 3,
    metered_dimensions: [],
    status: 'draft',
    created_at: '2026-02-01T00:00:00Z',
  },
];

export async function GET(request: NextRequest) {
  const auth = await guardPlatformAdmin();
  if (auth instanceof Response) return auth;

  const { searchParams } = request.nextUrl;
  const status = searchParams.get('status') ?? '';
  const page = parseInt(searchParams.get('page') ?? '1', 10);
  const pageSize = parseInt(searchParams.get('page_size') ?? '25', 10);

  // Build upstream query params
  const upstreamParams = new URLSearchParams();
  if (status) upstreamParams.set('status', status);
  upstreamParams.set('page', String(page));
  upstreamParams.set('page_size', String(pageSize));

  try {
    const upstreamUrl = `${TTP_BASE_URL}/api/ttp/plans?${upstreamParams}`;
    const res = await fetch(upstreamUrl, {
      headers: { 'Content-Type': 'application/json' },
      signal: AbortSignal.timeout(5000),
    });

    if (!res.ok) {
      return NextResponse.json(
        { error: `Upstream error: ${res.status}` },
        { status: res.status >= 500 ? 502 : res.status },
      );
    }

    const raw = await res.json();
    const parsed = PlanListResponseSchema.safeParse(raw);
    if (parsed.success) {
      return NextResponse.json(parsed.data);
    }

    // Upstream returned data in unexpected shape — pass through best-effort
    return NextResponse.json(raw);
  } catch {
    // TTP unavailable — return seed plan data so the UI still renders
    let filtered = SEED_PLANS;
    if (status) {
      filtered = filtered.filter((p) => p.status === status);
    }

    const total = filtered.length;
    const start = (page - 1) * pageSize;
    const paged = filtered.slice(start, start + pageSize);

    const fallback: PlanListResponse = {
      plans: paged,
      total,
      page,
      page_size: pageSize,
    };
    return NextResponse.json(fallback);
  }
}
