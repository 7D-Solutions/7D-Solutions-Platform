// ============================================================
// GET /api/tenants — BFF proxy to tenant-registry list endpoint
// POST /api/tenants — Create a new tenant via TTP
// Auth: requires platform_admin JWT in httpOnly cookie
// ============================================================
import { NextRequest, NextResponse } from 'next/server';
import { guardPlatformAdmin } from '@/lib/server/auth';
import { TENANT_REGISTRY_BASE_URL } from '@/lib/constants';
import { TenantListResponseSchema, CreateTenantRequestSchema } from '@/lib/api/types';
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

// ── POST /api/tenants ───────────────────────────────────────

export async function POST(request: NextRequest) {
  const auth = await guardPlatformAdmin();
  if (auth instanceof Response) return auth;

  let body: unknown;
  try {
    body = await request.json();
  } catch {
    return NextResponse.json({ error: 'Invalid JSON body' }, { status: 400 });
  }

  const parsed = CreateTenantRequestSchema.safeParse(body);
  if (!parsed.success) {
    const firstError = parsed.error.errors[0]?.message ?? 'Invalid request';
    return NextResponse.json({ error: firstError }, { status: 422 });
  }

  try {
    const upstreamUrl = `${TENANT_REGISTRY_BASE_URL}/api/tenants`;
    const res = await fetch(upstreamUrl, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(parsed.data),
      signal: AbortSignal.timeout(10000),
    });

    if (res.status === 404 || res.status === 405) {
      return NextResponse.json(
        { error: 'Tenant provisioning API not yet available. Use tenantctl CLI.' },
        { status: 501 },
      );
    }

    if (!res.ok) {
      const errBody = await res.json().catch(() => ({ error: `Upstream error: ${res.status}` }));
      return NextResponse.json(
        { error: errBody.error ?? `Upstream error: ${res.status}` },
        { status: res.status >= 500 ? 502 : res.status },
      );
    }

    const created = await res.json();
    return NextResponse.json(created, { status: 201 });
  } catch {
    return NextResponse.json(
      { error: 'Tenant provisioning API not yet available. Use tenantctl CLI.' },
      { status: 503 },
    );
  }
}
