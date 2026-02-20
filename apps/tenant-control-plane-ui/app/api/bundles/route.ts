// ============================================================
// GET /api/bundles — BFF proxy to TTP bundle catalog
// Forwards query params: status, page, page_size
// Auth: requires platform_admin JWT in httpOnly cookie
// Returns lightweight summaries (no composition per row).
// Falls back to seed data when TTP is unavailable.
// ============================================================
import { NextRequest, NextResponse } from 'next/server';
import { guardPlatformAdmin } from '@/lib/server/auth';
import { TTP_BASE_URL } from '@/lib/constants';
import { BundleListResponseSchema } from '@/lib/api/types';
import type { BundleListResponse, BundleSummary } from '@/lib/api/types';

// Seed bundles returned when TTP bundles endpoint is unavailable
const SEED_BUNDLES: BundleSummary[] = [
  {
    id: 'bundle-essential',
    name: 'Essential Features',
    status: 'active',
    entitlement_count: 5,
    created_at: '2026-01-01T00:00:00Z',
  },
  {
    id: 'bundle-analytics',
    name: 'Analytics Add-on',
    status: 'active',
    entitlement_count: 3,
    created_at: '2026-01-01T00:00:00Z',
  },
  {
    id: 'bundle-compliance',
    name: 'Compliance Suite',
    status: 'active',
    entitlement_count: 8,
    created_at: '2026-01-15T00:00:00Z',
  },
  {
    id: 'bundle-beta',
    name: 'Beta Features',
    status: 'draft',
    entitlement_count: 2,
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
    const upstreamUrl = `${TTP_BASE_URL}/api/ttp/bundles?${upstreamParams}`;
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
    const parsed = BundleListResponseSchema.safeParse(raw);
    if (parsed.success) {
      return NextResponse.json(parsed.data);
    }

    // Upstream returned data in unexpected shape — pass through best-effort
    return NextResponse.json(raw);
  } catch {
    // TTP unavailable — return seed data so the UI still renders
    let filtered = SEED_BUNDLES;
    if (status) {
      filtered = filtered.filter((b) => b.status === status);
    }

    const total = filtered.length;
    const start = (page - 1) * pageSize;
    const paged = filtered.slice(start, start + pageSize);

    const fallback: BundleListResponse = {
      bundles: paged,
      total,
      page,
      page_size: pageSize,
    };
    return NextResponse.json(fallback);
  }
}
