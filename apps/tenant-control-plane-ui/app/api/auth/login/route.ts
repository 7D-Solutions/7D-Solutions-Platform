// ============================================================
// POST /api/auth/login
// BFF guard — forwards credentials to identity-auth, sets
// httpOnly cookie on success. Never exposes JWT to JS.
// ============================================================
import { NextRequest, NextResponse } from 'next/server';
import { jwtVerify, decodeJwt } from 'jose';
import { AUTH_COOKIE_NAME, REQUIRED_ROLE, IDENTITY_AUTH_BASE_URL } from '@/lib/constants';

export async function POST(req: NextRequest) {
  let body: { email?: string; password?: string };
  try {
    body = await req.json();
  } catch {
    return NextResponse.json({ error: 'Invalid request body' }, { status: 400 });
  }

  const { email, password } = body;
  if (!email || !password) {
    return NextResponse.json({ error: 'Email and password are required' }, { status: 400 });
  }

  // Forward credentials to identity-auth service
  let upstreamRes: Response;
  try {
    upstreamRes = await fetch(`${IDENTITY_AUTH_BASE_URL}/auth/login`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ email, password }),
    });
  } catch {
    return NextResponse.json({ error: 'Auth service unavailable' }, { status: 503 });
  }

  if (!upstreamRes.ok) {
    if (upstreamRes.status === 401) {
      return NextResponse.json({ error: 'Invalid credentials' }, { status: 401 });
    }
    return NextResponse.json({ error: 'Login failed' }, { status: upstreamRes.status });
  }

  let data: { token?: string };
  try {
    data = await upstreamRes.json();
  } catch {
    return NextResponse.json({ error: 'Invalid upstream response' }, { status: 502 });
  }

  const token = data.token;
  if (!token) {
    return NextResponse.json({ error: 'No token in response' }, { status: 502 });
  }

  // Verify the token has the required platform_admin role before accepting it
  let hasPlatformAdmin = false;
  const jwtSecret = process.env.JWT_SECRET;
  try {
    if (jwtSecret) {
      const { payload } = await jwtVerify(token, new TextEncoder().encode(jwtSecret));
      const roles = (payload.roles as string[] | undefined) ?? [];
      hasPlatformAdmin = roles.includes(REQUIRED_ROLE);
    } else {
      // Dev-only fallback: decode without verification
      const payload = decodeJwt(token);
      const roles = (payload.roles as string[] | undefined) ?? [];
      hasPlatformAdmin = roles.includes(REQUIRED_ROLE);
    }
  } catch {
    return NextResponse.json({ error: 'Invalid token' }, { status: 401 });
  }

  if (!hasPlatformAdmin) {
    return NextResponse.json(
      { error: 'Forbidden — platform_admin role required' },
      { status: 403 },
    );
  }

  const res = NextResponse.json({ ok: true });
  res.cookies.set(AUTH_COOKIE_NAME, token, {
    httpOnly: true,
    secure: process.env.NODE_ENV === 'production',
    sameSite: 'lax',
    path: '/',
    // Hard ceiling of 8 hours regardless of JWT expiry
    maxAge: 8 * 60 * 60,
  });
  return res;
}
