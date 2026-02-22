// ============================================================
// GET /api/payments/checkout-sessions/[session_id]/status
// BFF proxy: lightweight status poll for client-side polling.
// Returns only {session_id, status} — no client_secret.
//
// No staff auth required — the session_id UUID is the access token.
// State machine values: created | presented | completed | failed | canceled | expired
// ============================================================
import { NextRequest, NextResponse } from 'next/server';
import { PAYMENTS_BASE_URL } from '@/lib/constants';

export interface CheckoutSessionStatusPoll {
  session_id: string;
  status: string;
}

export async function GET(
  _req: NextRequest,
  { params }: { params: Promise<{ session_id: string }> },
) {
  const { session_id } = await params;

  if (!/^[0-9a-f-]{36}$/i.test(session_id)) {
    return NextResponse.json({ error: 'Invalid session ID' }, { status: 400 });
  }

  let body: CheckoutSessionStatusPoll;
  try {
    const res = await fetch(
      `${PAYMENTS_BASE_URL}/api/payments/checkout-sessions/${session_id}/status`,
      {
        cache: 'no-store',
        signal: AbortSignal.timeout(5000),
      },
    );

    if (res.status === 404) {
      return NextResponse.json({ error: 'Session not found' }, { status: 404 });
    }
    if (!res.ok) {
      return NextResponse.json(
        { error: `Upstream error: ${res.status}` },
        { status: res.status >= 500 ? 502 : res.status },
      );
    }

    body = (await res.json()) as CheckoutSessionStatusPoll;
  } catch {
    return NextResponse.json({ error: 'Payments service unavailable' }, { status: 503 });
  }

  return NextResponse.json(body);
}
