// ============================================================
// POST /api/tenants/[tenant_id]/rbac/revoke — Revoke role from user
// Proxies to identity-auth; seed-mode returns success.
// Auth: requires platform_admin JWT in httpOnly cookie
// ============================================================
import { NextRequest, NextResponse } from 'next/server';
import { guardPlatformAdmin } from '@/lib/server/auth';
import { IDENTITY_AUTH_BASE_URL } from '@/lib/constants';
import { RbacChangeRequestSchema } from '@/lib/api/types';

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

  const parsed = RbacChangeRequestSchema.safeParse(body);
  if (!parsed.success) {
    return NextResponse.json(
      { error: 'Validation failed', issues: parsed.error.issues },
      { status: 422 },
    );
  }

  // Ensure the action is revoke
  if (parsed.data.action !== 'revoke') {
    return NextResponse.json(
      { error: 'Use /rbac/grant for grant actions' },
      { status: 400 },
    );
  }

  const { user_id, role_id } = parsed.data;

  try {
    const upstreamUrl =
      `${IDENTITY_AUTH_BASE_URL}/api/tenants/${encodeURIComponent(tenant_id)}/rbac/revoke`;
    const res = await fetch(upstreamUrl, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ user_id, role_id }),
      signal: AbortSignal.timeout(5000),
    });

    if (res.ok) {
      return NextResponse.json({ ok: true });
    }

    // Non-ok responses — fall through to seed-mode
  } catch {
    // identity-auth unavailable — fall through to seed-mode
  }

  // Seed-mode: simulate success
  return NextResponse.json({ ok: true });
}
