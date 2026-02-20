// ============================================================
// GET /api/notifications — BFF proxy to notifications backend
// Returns persisted notification list + unread count.
// When the notifications backend is unavailable, returns
// deterministic empty state so the UI still renders.
// Auth: requires platform_admin JWT in httpOnly cookie
// ============================================================
import { NextResponse } from 'next/server';
import { guardPlatformAdmin } from '@/lib/server/auth';
import { NOTIFICATIONS_BASE_URL } from '@/lib/constants';
import { NotificationListResponseSchema } from '@/lib/api/types';
import type { NotificationListResponse } from '@/lib/api/types';

const EMPTY_RESPONSE: NotificationListResponse = {
  notifications: [],
  unread_count: 0,
};

export async function GET() {
  const auth = await guardPlatformAdmin();
  if (auth instanceof Response) return auth;

  try {
    const res = await fetch(
      `${NOTIFICATIONS_BASE_URL}/api/notifications?user_id=${auth.sub}`,
      {
        headers: { 'Content-Type': 'application/json' },
        signal: AbortSignal.timeout(3000),
      },
    );

    if (!res.ok) {
      // Backend returned an error — return empty state
      return NextResponse.json(EMPTY_RESPONSE);
    }

    const raw = await res.json();
    const parsed = NotificationListResponseSchema.safeParse(raw);
    if (parsed.success) {
      return NextResponse.json(parsed.data);
    }

    // Unexpected shape — return empty state
    return NextResponse.json(EMPTY_RESPONSE);
  } catch {
    // Notifications backend unavailable — return deterministic empty state
    return NextResponse.json(EMPTY_RESPONSE);
  }
}
