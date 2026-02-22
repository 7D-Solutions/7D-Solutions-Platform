// ============================================================
// TCP UI — Middleware
// Protects all authenticated routes. Requires valid JWT + platform_admin.
// Public routes: /login, /forbidden, /api/*, /_next/*, /favicon.ico
// ============================================================
import { NextRequest, NextResponse } from 'next/server';
import { decodeJwt } from 'jose';
import { AUTH_COOKIE_NAME, REQUIRED_ROLE } from '@/lib/constants';

/** Routes that do not require authentication */
function isPublicRoute(pathname: string): boolean {
  if (pathname === '/login' || pathname.startsWith('/login')) return true;
  if (pathname === '/forbidden' || pathname.startsWith('/forbidden')) return true;
  // Hosted pay portal — customer-facing, no staff auth required
  if (pathname.startsWith('/pay/')) return true;
  if (pathname.startsWith('/api/')) return true;
  if (pathname.startsWith('/_next/')) return true;
  if (pathname === '/favicon.ico') return true;
  return false;
}

export function middleware(request: NextRequest) {
  const { pathname } = request.nextUrl;

  if (isPublicRoute(pathname)) {
    return NextResponse.next();
  }

  const token = request.cookies.get(AUTH_COOKIE_NAME)?.value;

  if (!token) {
    const loginUrl = new URL('/login', request.url);
    loginUrl.searchParams.set('redirect', pathname);
    return NextResponse.redirect(loginUrl);
  }

  try {
    const payload = decodeJwt(token);

    // Check expiry
    const now = Math.floor(Date.now() / 1000);
    if (payload.exp && payload.exp < now) {
      const loginUrl = new URL('/login', request.url);
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
      return NextResponse.redirect(new URL('/forbidden', request.url));
    }

    return NextResponse.next();
  } catch {
    const loginUrl = new URL('/login', request.url);
    loginUrl.searchParams.set('redirect', pathname);
    return NextResponse.redirect(loginUrl);
  }
}

export const config = {
  matcher: ['/((?!_next/static|_next/image|favicon\\.ico).*)'],
};
