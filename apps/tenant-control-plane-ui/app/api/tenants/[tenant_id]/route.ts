// ============================================================
// GET /api/tenants/[tenant_id] — BFF proxy to tenant-registry detail
// Returns tenant metadata. Falls back to seed data when registry is down.
// Auth: requires platform_admin JWT in httpOnly cookie
// ============================================================
import { NextRequest, NextResponse } from 'next/server';
import { guardPlatformAdmin } from '@/lib/server/auth';
import { TENANT_REGISTRY_BASE_URL } from '@/lib/constants';
import { TenantDetailSchema } from '@/lib/api/types';
import type { TenantDetail } from '@/lib/api/types';

export async function GET(
  _request: NextRequest,
  { params }: { params: Promise<{ tenant_id: string }> }
) {
  const auth = await guardPlatformAdmin();
  if (auth instanceof Response) return auth;

  const { tenant_id } = await params;

  try {
    const upstreamUrl = `${TENANT_REGISTRY_BASE_URL}/api/tenants/${encodeURIComponent(tenant_id)}`;
    const res = await fetch(upstreamUrl, {
      headers: { 'Content-Type': 'application/json' },
      signal: AbortSignal.timeout(5000),
    });

    if (!res.ok) {
      if (res.status === 404) {
        return NextResponse.json(
          { error: 'Tenant not found' },
          { status: 404 },
        );
      }
      return NextResponse.json(
        { error: `Upstream error: ${res.status}` },
        { status: res.status >= 500 ? 502 : res.status },
      );
    }

    const raw = await res.json();
    const parsed = TenantDetailSchema.safeParse(raw);
    if (parsed.success) {
      return NextResponse.json(parsed.data);
    }
    return NextResponse.json(raw);
  } catch {
    // Registry unavailable — return seed data so the UI still renders
    const fallback: TenantDetail = {
      id: tenant_id,
      name: `Tenant ${tenant_id}`,
      status: 'unknown',
      plan: 'Unknown',
    };
    return NextResponse.json(fallback);
  }
}
