// ============================================================
// GET /api/payments/checkout-sessions/[session_id]
// BFF proxy to Payments service checkout session read endpoint.
// Used by the hosted pay page (/pay/[session_id]) for client-side
// status polling after payment confirmation.
//
// No staff auth required — the session_id UUID is the access token.
// Validates return_url/cancel_url are absolute HTTPS before returning.
// ============================================================
import { NextRequest, NextResponse } from 'next/server';
import { PAYMENTS_BASE_URL } from '@/lib/constants';

export interface CheckoutSessionResponse {
  session_id: string;
  status: string;
  payment_intent_id: string;
  invoice_id: string;
  tenant_id: string;
  amount: number;
  currency: string;
  client_secret: string;
  return_url: string | null;
  cancel_url: string | null;
}

/** Validate that a URL is absolute HTTPS with no injection characters. */
function isValidHttpsUrl(url: string): boolean {
  if (!url.startsWith('https://')) return false;
  if (url.length > 2048) return false;
  if (/[\u0000-\u001f]/.test(url)) return false;
  return true;
}

export async function GET(
  _req: NextRequest,
  { params }: { params: Promise<{ session_id: string }> },
) {
  const { session_id } = await params;

  // Basic UUID format guard — prevent trivial path traversal
  if (!/^[0-9a-f-]{36}$/i.test(session_id)) {
    return NextResponse.json({ error: 'Invalid session ID' }, { status: 400 });
  }

  let session: CheckoutSessionResponse;
  try {
    const res = await fetch(
      `${PAYMENTS_BASE_URL}/api/payments/checkout-sessions/${session_id}`,
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

    session = (await res.json()) as CheckoutSessionResponse;
  } catch {
    return NextResponse.json({ error: 'Payments service unavailable' }, { status: 503 });
  }

  // Defense in depth: validate stored URLs before forwarding to client
  if (session.return_url && !isValidHttpsUrl(session.return_url)) {
    session = { ...session, return_url: null };
  }
  if (session.cancel_url && !isValidHttpsUrl(session.cancel_url)) {
    session = { ...session, cancel_url: null };
  }

  return NextResponse.json(session);
}
