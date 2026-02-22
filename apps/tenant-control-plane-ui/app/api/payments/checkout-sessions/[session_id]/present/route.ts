// ============================================================
// POST /api/payments/checkout-sessions/[session_id]/present
// BFF proxy: idempotent created → presented transition.
// Called by the hosted pay page on load to record that the
// customer has seen the payment form.
//
// No staff auth required — the session_id UUID is the access token.
// Idempotent: already-presented or terminal sessions return 200.
// ============================================================
import { NextRequest, NextResponse } from 'next/server';
import { PAYMENTS_BASE_URL } from '@/lib/constants';

export async function POST(
  _req: NextRequest,
  { params }: { params: Promise<{ session_id: string }> },
) {
  const { session_id } = await params;

  if (!/^[0-9a-f-]{36}$/i.test(session_id)) {
    return NextResponse.json({ error: 'Invalid session ID' }, { status: 400 });
  }

  try {
    const res = await fetch(
      `${PAYMENTS_BASE_URL}/api/payments/checkout-sessions/${session_id}/present`,
      {
        method: 'POST',
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
  } catch {
    return NextResponse.json({ error: 'Payments service unavailable' }, { status: 503 });
  }

  return new NextResponse(null, { status: 200 });
}
