// ============================================================
// TCP UI — Middleware
// Protects all /app/** routes. Requires valid JWT + platform_admin.
// Login page (/app/login) is exempt.
// ============================================================
import { NextRequest, NextResponse } from 'next/server';
import { decodeJwt } from 'jose';
import { AUTH_COOKIE_NAME, REQUIRED_ROLE } from '@/lib/constants';

export function middleware(request: NextRequest) {
  const { pathname } = request.nextUrl;

  // Only protect /app/** routes
  if (!pathname.startsWith('/app')) {
    return NextResponse.next();
  }

  // Login page is public
  if (pathname === '/app/login' || pathname.startsWith('/app/login')) {
    return NextResponse.next();
  }

  const token = request.cookies.get(AUTH_COOKIE_NAME)?.value;

  if (!token) {
    const loginUrl = new URL('/app/login', request.url);
    loginUrl.searchParams.set('redirect', pathname);
    return NextResponse.redirect(loginUrl);
  }

  try {
    const payload = decodeJwt(token);

    // Check expiry
    const now = Math.floor(Date.now() / 1000);
    if (payload.exp && payload.exp < now) {
      const loginUrl = new URL('/app/login', request.url);
      loginUrl.searchParams.set('redirect', pathname);
      loginUrl.searchParams.set('reason', 'expired');
      return NextResponse.redirect(loginUrl);
    }

    // Check platform_admin in roles or perms
    const roles = (payload.roles as string[] | undefined) ?? [];
    const perms = (payload.perms as string[] | undefined) ?? [];
    const allClaims = [...roles, ...perms];
    const hasPlatformAdmin = allClaims.some(
      (c) => c === REQUIRED_ROLE || c.startsWith('platform_admin')
    );

    if (!hasPlatformAdmin) {
      return NextResponse.redirect(new URL('/app/login?reason=forbidden', request.url));
    }

    return NextResponse.next();
  } catch {
    const loginUrl = new URL('/app/login', request.url);
    loginUrl.searchParams.set('redirect', pathname);
    return NextResponse.redirect(loginUrl);
  }
}

export const config = {
  matcher: ['/app/:path*'],
};
