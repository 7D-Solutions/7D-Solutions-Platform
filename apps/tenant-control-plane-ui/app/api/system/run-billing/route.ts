// ============================================================
// POST /api/system/run-billing — BFF proxy to TTP billing run
// Triggers an immediate billing cycle. Tenant ID is optional;
// if omitted the backend runs billing for all tenants.
// Auth: requires platform_admin JWT in httpOnly cookie
// ============================================================
import { NextRequest, NextResponse } from 'next/server';
import { guardPlatformAdmin } from '@/lib/server/auth';
import { TTP_BASE_URL } from '@/lib/constants';
import { RunBillingRequestSchema } from '@/lib/api/types';
import type { AdminToolResult } from '@/lib/api/types';

export async function POST(request: NextRequest) {
  const auth = await guardPlatformAdmin();
  if (auth instanceof Response) return auth;

  const body = await request.json().catch(() => null);
  const parsed = RunBillingRequestSchema.safeParse(body);
  if (!parsed.success) {
    return NextResponse.json(
      { error: parsed.error.issues[0]?.message ?? 'Invalid request' },
      { status: 400 },
    );
  }

  const { tenant_id, reason } = parsed.data;

  try {
    const upstreamUrl = `${TTP_BASE_URL}/api/billing/run`;
    const payload: Record<string, string> = { reason };
    if (tenant_id) payload.tenant_id = tenant_id;

    const res = await fetch(upstreamUrl, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(payload),
      signal: AbortSignal.timeout(10_000),
    });

    if (res.ok) {
      const data = await res.json().catch(() => ({}));
      const result: AdminToolResult = {
        ok: true,
        message: data.message ?? 'Billing run completed successfully.',
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
    // TTP unreachable — seed-mode: return not-available
    const result: AdminToolResult = {
      ok: false,
      not_available: true,
      message: 'Billing service is not available in this environment.',
    };
    return NextResponse.json(result);
  }
}
