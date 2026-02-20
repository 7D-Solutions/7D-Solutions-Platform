// ============================================================
// POST /api/tenants/[tenant_id]/support-sessions/start
// BFF — starts a support session for the given tenant.
// Proxies to identity-auth for support JWT issuance.
// Stores support token in a separate httpOnly cookie.
// Auth: requires existing platform_admin JWT in httpOnly cookie
// ============================================================
import { NextRequest, NextResponse } from 'next/server';
import { SignJWT } from 'jose';
import { guardPlatformAdmin, getStaffClaims } from '@/lib/server/auth';
import { IDENTITY_AUTH_BASE_URL, SUPPORT_SESSION_COOKIE_NAME } from '@/lib/constants';

export async function POST(
  req: NextRequest,
  { params }: { params: Promise<{ tenant_id: string }> },
) {
  const auth = await guardPlatformAdmin();
  if (auth instanceof Response) return auth;

  const { tenant_id } = await params;

  let body: { reason?: string };
  try {
    body = await req.json();
  } catch {
    return NextResponse.json({ error: 'Invalid request body' }, { status: 400 });
  }

  const { reason } = body;
  if (!reason || !reason.trim()) {
    return NextResponse.json({ error: 'Reason is required' }, { status: 400 });
  }

  if (reason.length > 500) {
    return NextResponse.json({ error: 'Reason must be 500 characters or fewer' }, { status: 400 });
  }

  const claims = await getStaffClaims();
  if (!claims?.email) {
    return NextResponse.json({ error: 'Unable to read session' }, { status: 401 });
  }

  // Try to get a support JWT from identity-auth
  try {
    const res = await fetch(
      `${IDENTITY_AUTH_BASE_URL}/auth/support-session`,
      {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          staff_email: claims.email,
          staff_id: claims.sub,
          tenant_id,
          reason,
        }),
        signal: AbortSignal.timeout(5000),
      },
    );

    if (res.ok) {
      const data = await res.json();
      if (data.token) {
        const response = NextResponse.json({
          ok: true,
          actor_type: 'support',
          tenant_id,
        });
        response.cookies.set(SUPPORT_SESSION_COOKIE_NAME, data.token, {
          httpOnly: true,
          secure: process.env.NODE_ENV === 'production',
          sameSite: 'lax',
          path: '/',
          maxAge: 60 * 60, // 1 hour max for support sessions
        });
        return response;
      }
    }
    // Fall through to seed-mode if backend doesn't provide a token
  } catch {
    // identity-auth unavailable — fall through to seed-mode
  }

  // Seed-mode: self-issue a support session JWT
  const secret = new TextEncoder().encode(process.env.JWT_SECRET ?? 'dev-support-secret');
  const supportToken = await new SignJWT({
    sub: claims.sub,
    email: claims.email,
    actor_type: 'support',
    tenant_id,
    reason,
    roles: claims.roles ?? [],
  })
    .setProtectedHeader({ alg: 'HS256' })
    .setIssuedAt()
    .setExpirationTime('1h')
    .sign(secret);

  const response = NextResponse.json({
    ok: true,
    actor_type: 'support',
    tenant_id,
  });
  response.cookies.set(SUPPORT_SESSION_COOKIE_NAME, supportToken, {
    httpOnly: true,
    secure: process.env.NODE_ENV === 'production',
    sameSite: 'lax',
    path: '/',
    maxAge: 60 * 60,
  });
  return response;
}
