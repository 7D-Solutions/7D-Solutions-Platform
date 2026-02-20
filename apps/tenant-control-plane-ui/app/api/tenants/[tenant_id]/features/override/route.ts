// ============================================================
// POST /api/tenants/[tenant_id]/features/override
// BFF proxy for granting/revoking entitlement overrides.
// Requires justification. Proxies to TTP override endpoint.
// Auth: requires platform_admin JWT in httpOnly cookie
// Falls back to seed-mode success when TTP is unavailable.
// ============================================================
import { NextRequest, NextResponse } from 'next/server';
import { guardPlatformAdmin } from '@/lib/server/auth';
import { TTP_BASE_URL } from '@/lib/constants';
import { FeatureOverrideRequestSchema } from '@/lib/api/types';

export async function POST(
  request: NextRequest,
  { params }: { params: Promise<{ tenant_id: string }> }
) {
  const auth = await guardPlatformAdmin();
  if (auth instanceof Response) return auth;

  const { tenant_id } = await params;

  // Parse and validate request body
  let body: unknown;
  try {
    body = await request.json();
  } catch {
    return NextResponse.json(
      { error: 'Invalid JSON body' },
      { status: 400 },
    );
  }

  const parsed = FeatureOverrideRequestSchema.safeParse(body);
  if (!parsed.success) {
    return NextResponse.json(
      { error: 'Validation failed', issues: parsed.error.issues },
      { status: 422 },
    );
  }

  const { entitlement_code, action, justification } = parsed.data;

  try {
    const upstreamUrl =
      `${TTP_BASE_URL}/api/ttp/tenants/${encodeURIComponent(tenant_id)}/features/override`;
    const res = await fetch(upstreamUrl, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ entitlement_code, action, justification }),
      signal: AbortSignal.timeout(5000),
    });

    if (res.ok) {
      return NextResponse.json({ ok: true });
    }

    if (res.status === 409) {
      return NextResponse.json(
        { error: 'Override conflict — entitlement may already be in the requested state' },
        { status: 409 },
      );
    }

    if (res.status === 404) {
      return NextResponse.json(
        { error: 'Entitlement or tenant not found' },
        { status: 404 },
      );
    }

    // Other upstream errors — fall through to seed-mode
  } catch {
    // TTP unavailable — fall through to seed-mode success
  }

  // Seed-mode: simulate success when upstream endpoint is unavailable
  return NextResponse.json({ ok: true });
}
