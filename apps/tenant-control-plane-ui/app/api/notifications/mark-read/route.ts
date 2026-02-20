// ============================================================
// POST /api/notifications/mark-read — mark notifications as read
// Accepts { ids: string[] } to mark specific items, or { all: true }
// to mark everything. Proxies to notifications backend when available;
// returns success deterministically when backend is unavailable.
// Auth: requires platform_admin JWT in httpOnly cookie
// ============================================================
import { NextRequest, NextResponse } from 'next/server';
import { guardPlatformAdmin } from '@/lib/server/auth';
import { NOTIFICATIONS_BASE_URL } from '@/lib/constants';
import { MarkReadRequestSchema } from '@/lib/api/types';

export async function POST(request: NextRequest) {
  const auth = await guardPlatformAdmin();
  if (auth instanceof Response) return auth;

  const body = await request.json();
  const parsed = MarkReadRequestSchema.safeParse(body);
  if (!parsed.success) {
    return NextResponse.json(
      { error: 'Invalid request: provide { ids: string[] } or { all: true }' },
      { status: 400 },
    );
  }

  try {
    const res = await fetch(
      `${NOTIFICATIONS_BASE_URL}/api/notifications/mark-read`,
      {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ ...parsed.data, user_id: auth.sub }),
        signal: AbortSignal.timeout(3000),
      },
    );

    if (res.ok) {
      const data = await res.json();
      return NextResponse.json(data);
    }
  } catch {
    // Notifications backend unavailable — no-op
  }

  // Backend unavailable or errored — return success deterministically
  return NextResponse.json({ success: true });
}
