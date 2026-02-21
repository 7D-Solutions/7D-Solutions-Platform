// ============================================================
// POST /api/system/run-billing — BFF proxy to TTP billing run
// Triggers an immediate billing cycle for a single tenant.
// Auth: requires platform_admin JWT in httpOnly cookie
// ============================================================
import { NextRequest, NextResponse } from 'next/server';
import { guardPlatformAdmin } from '@/lib/server/auth';
import { TTP_BASE_URL } from '@/lib/constants';
import { RunBillingRequestSchema } from '@/lib/api/types';
import type { AdminToolResult } from '@/lib/api/types';

function currentBillingPeriod(): string {
  const now = new Date();
  const yyyy = now.getFullYear();
  const mm = String(now.getMonth() + 1).padStart(2, '0');
  return `${yyyy}-${mm}`;
}

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

  const { tenant_id, billing_period } = parsed.data;

  // TTP requires a concrete tenant_id — the "all tenants" variant is not yet
  // supported by the TTP billing-runs endpoint.
  if (!tenant_id) {
    return NextResponse.json(
      { ok: false, message: 'Tenant ID is required for billing runs.' },
      { status: 400 },
    );
  }

  const period = billing_period ?? currentBillingPeriod();
  const idempotency_key = `bff-${tenant_id}-${period}-${Date.now()}`;

  try {
    const upstreamUrl = `${TTP_BASE_URL}/api/ttp/billing-runs`;
    const payload = { tenant_id, billing_period: period, idempotency_key };

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
        message: data.was_noop
          ? `Billing run already completed for ${period} (no-op).`
          : `Billing run completed for ${period}.`,
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
