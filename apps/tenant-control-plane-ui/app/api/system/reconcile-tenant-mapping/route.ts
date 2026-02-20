// ============================================================
// POST /api/system/reconcile-tenant-mapping — BFF proxy
// Triggers a reconciliation of the tenant mapping for a
// specific tenant. Tenant ID is required.
// Auth: requires platform_admin JWT in httpOnly cookie
// ============================================================
import { NextRequest, NextResponse } from 'next/server';
import { guardPlatformAdmin } from '@/lib/server/auth';
import { TENANT_REGISTRY_BASE_URL } from '@/lib/constants';
import { ReconcileMappingRequestSchema } from '@/lib/api/types';
import type { AdminToolResult } from '@/lib/api/types';

export async function POST(request: NextRequest) {
  const auth = await guardPlatformAdmin();
  if (auth instanceof Response) return auth;

  const body = await request.json().catch(() => null);
  const parsed = ReconcileMappingRequestSchema.safeParse(body);
  if (!parsed.success) {
    return NextResponse.json(
      { error: parsed.error.issues[0]?.message ?? 'Invalid request' },
      { status: 400 },
    );
  }

  const { tenant_id, reason } = parsed.data;

  try {
    const upstreamUrl =
      `${TENANT_REGISTRY_BASE_URL}/api/tenants/${encodeURIComponent(tenant_id)}/reconcile-mapping`;

    const res = await fetch(upstreamUrl, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ reason }),
      signal: AbortSignal.timeout(10_000),
    });

    if (res.ok) {
      const data = await res.json().catch(() => ({}));
      const result: AdminToolResult = {
        ok: true,
        message: data.message ?? 'Tenant mapping reconciled successfully.',
      };
      return NextResponse.json(result);
    }

    if (res.status === 404 || res.status === 501) {
      const data = await res.json().catch(() => ({}));
      const result: AdminToolResult = {
        ok: false,
        not_available: true,
        message: data.error ?? `Not available in this environment (HTTP ${res.status})`,
      };
      return NextResponse.json(result);
    }

    const data = await res.json().catch(() => ({}));
    return NextResponse.json(
      { ok: false, message: data.error ?? `Upstream error (HTTP ${res.status})` },
      { status: res.status },
    );
  } catch {
    // Tenant registry unreachable — seed-mode: return not-available
    const result: AdminToolResult = {
      ok: false,
      not_available: true,
      message: 'Tenant registry is not available in this environment.',
    };
    return NextResponse.json(result);
  }
}
