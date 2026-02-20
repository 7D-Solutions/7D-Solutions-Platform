// ============================================================
// GET /api/tenants — BFF proxy to tenant-registry list endpoint
// Forwards query params: search, status, plan, app_id, page, page_size
// Auth: requires platform_admin JWT in httpOnly cookie
// ============================================================
import { NextRequest, NextResponse } from 'next/server';
import { guardPlatformAdmin } from '@/lib/server/auth';
import { TENANT_REGISTRY_BASE_URL } from '@/lib/constants';
import { TenantListResponseSchema } from '@/lib/api/types';
import type { TenantListResponse } from '@/lib/api/types';

export async function GET(request: NextRequest) {
  const auth = await guardPlatformAdmin();
  if (auth instanceof Response) return auth;

  const { searchParams } = request.nextUrl;
  const search = searchParams.get('search') ?? '';
  const status = searchParams.get('status') ?? '';
  const plan = searchParams.get('plan') ?? '';
  const appId = searchParams.get('app_id') ?? '';
  const page = parseInt(searchParams.get('page') ?? '1', 10);
  const pageSize = parseInt(searchParams.get('page_size') ?? '25', 10);

  // Build upstream query params
  const upstreamParams = new URLSearchParams();
  if (search) upstreamParams.set('search', search);
  if (status) upstreamParams.set('status', status);
  if (plan) upstreamParams.set('plan', plan);
  if (appId) upstreamParams.set('app_id', appId);
  upstreamParams.set('page', String(page));
  upstreamParams.set('page_size', String(pageSize));

  try {
    const upstreamUrl = `${TENANT_REGISTRY_BASE_URL}/api/tenants?${upstreamParams}`;
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
    const parsed = TenantListResponseSchema.safeParse(raw);
    if (parsed.success) {
      return NextResponse.json(parsed.data);
    }

    // Upstream returned data but in an unexpected shape — pass through best-effort
    return NextResponse.json(raw);
  } catch {
    // Tenant-registry unavailable — return empty list so the UI still renders
    const fallback: TenantListResponse = {
      tenants: [],
      total: 0,
      page,
      page_size: pageSize,
    };
    return NextResponse.json(fallback);
  }
}
