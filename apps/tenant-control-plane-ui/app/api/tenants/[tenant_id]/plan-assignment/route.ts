// ============================================================
// POST /api/tenants/[tenant_id]/plan-assignment — BFF proxy to TTP
// Assigns (or changes) a tenant's plan with an effective date.
// Auth: requires platform_admin JWT in httpOnly cookie
// Seed-mode: simulates success when TTP is unavailable.
// ============================================================
import { NextRequest, NextResponse } from 'next/server';
import { guardPlatformAdmin } from '@/lib/server/auth';
import { TTP_BASE_URL } from '@/lib/constants';
import { PlanAssignmentRequestSchema } from '@/lib/api/types';

export async function POST(
  request: NextRequest,
  { params }: { params: Promise<{ tenant_id: string }> }
) {
  const auth = await guardPlatformAdmin();
  if (auth instanceof Response) return auth;

  const { tenant_id } = await params;

  let body: unknown;
  try {
    body = await request.json();
  } catch {
    return NextResponse.json({ error: 'Invalid JSON body' }, { status: 400 });
  }

  const parsed = PlanAssignmentRequestSchema.safeParse(body);
  if (!parsed.success) {
    const firstError = parsed.error.issues[0]?.message ?? 'Validation failed';
    return NextResponse.json({ error: firstError }, { status: 400 });
  }

  const { plan_id, effective_date } = parsed.data;

  try {
    const upstreamUrl =
      `${TTP_BASE_URL}/api/ttp/tenants/${encodeURIComponent(tenant_id)}/plan`;
    const res = await fetch(upstreamUrl, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ plan_id, effective_date }),
      signal: AbortSignal.timeout(5000),
    });

    if (res.ok) {
      const data = await res.json().catch(() => ({ ok: true }));
      return NextResponse.json(data);
    }

    if (res.status === 404) {
      return NextResponse.json({ error: 'Tenant or plan not found' }, { status: 404 });
    }

    if (res.status === 409) {
      return NextResponse.json(
        { error: 'Tenant is already on this plan' },
        { status: 409 },
      );
    }

    if (res.status === 422) {
      const errBody = await res.json().catch(() => ({ error: 'Validation failed' }));
      return NextResponse.json(
        { error: errBody.error ?? 'Validation failed' },
        { status: 422 },
      );
    }

    // Other upstream errors — fall through to seed-mode
  } catch {
    // TTP unavailable — fall through to seed-mode
  }

  // Seed-mode: simulate success when upstream is unavailable
  return NextResponse.json({ ok: true, plan_id, effective_date });
}
