// ============================================================
// /pay/[session_id] — Hosted pay portal (public, no staff auth)
// Customers land here from invoice payment links.
//
// Server component: fetches session data directly from Payments
// service (server → server). Passes to PaymentForm client component.
//
// Security invariants:
//   - client_secret never appears in URLs
//   - return_url / cancel_url validated as absolute HTTPS at Rust level
//   - No session data leaked in HTML beyond what's needed for Tilled.js
// ============================================================
import type { Metadata } from 'next';
import { PaymentForm } from './PaymentForm';
import type { SessionData } from './PaymentForm';
import { PAYMENTS_BASE_URL } from '@/lib/constants';

interface PageProps {
  params: Promise<{ session_id: string }>;
}

export const metadata: Metadata = {
  title: 'Secure Payment — 7D Solutions',
  description: 'Complete your secure payment',
};

// Always fetch fresh — payment status changes
export const dynamic = 'force-dynamic';

async function fetchSession(sessionId: string): Promise<SessionData | null> {
  // Basic UUID format guard before making upstream call
  if (!/^[0-9a-f-]{36}$/i.test(sessionId)) return null;

  try {
    const res = await fetch(
      `${PAYMENTS_BASE_URL}/api/payments/checkout-sessions/${sessionId}`,
      { cache: 'no-store', signal: AbortSignal.timeout(5000) },
    );
    if (!res.ok) return null;
    const data = await res.json();
    return data as SessionData;
  } catch {
    return null;
  }
}

function formatAmount(amount: number, currency: string): string {
  return new Intl.NumberFormat('en-US', {
    style: 'currency',
    currency: currency.toUpperCase(),
  }).format(amount / 100);
}

export default async function PayPage({ params }: PageProps) {
  const { session_id } = await params;
  const session = await fetchSession(session_id);

  if (!session) {
    return (
      <div
        className="min-h-screen bg-gray-50 flex items-center justify-center px-4"
        data-testid="pay-not-found"
      >
        <div className="max-w-md w-full bg-white rounded-2xl shadow-sm border border-gray-200 p-8 text-center">
          <div className="text-gray-400 text-5xl mb-4">✕</div>
          <h1 className="text-xl font-semibold text-gray-900 mb-2">Payment link not found</h1>
          <p className="text-gray-500 text-sm">
            This payment link is invalid or has expired. Please contact support if you need
            assistance.
          </p>
        </div>
      </div>
    );
  }

  // Already-terminal sessions: show status instead of form
  if (session.status === 'succeeded') {
    return (
      <div
        className="min-h-screen bg-gray-50 flex items-center justify-center px-4"
        data-testid="pay-portal"
      >
        <div className="max-w-md w-full bg-white rounded-2xl shadow-sm border border-gray-200 p-8 text-center">
          <div className="text-green-500 text-5xl mb-4">✓</div>
          <h1 className="text-xl font-semibold text-gray-900 mb-2">Payment complete</h1>
          <p className="text-gray-500 text-sm">This payment has already been processed.</p>
        </div>
      </div>
    );
  }

  if (session.status === 'cancelled' || session.status === 'failed') {
    return (
      <div
        className="min-h-screen bg-gray-50 flex items-center justify-center px-4"
        data-testid="pay-portal"
      >
        <div className="max-w-md w-full bg-white rounded-2xl shadow-sm border border-gray-200 p-8 text-center">
          <div className="text-red-400 text-5xl mb-4">✕</div>
          <h1 className="text-xl font-semibold text-gray-900 mb-2">Payment {session.status}</h1>
          <p className="text-gray-500 text-sm">
            This payment link is no longer active. Please request a new payment link if needed.
          </p>
        </div>
      </div>
    );
  }

  // Read Tilled publishable credentials from server env
  // NEXT_PUBLIC_ prefix makes them available to client components too
  const tilledPublishableKey = process.env.NEXT_PUBLIC_TILLED_PUBLISHABLE_KEY ?? null;
  const tilledAccountId = process.env.NEXT_PUBLIC_TILLED_ACCOUNT_ID ?? null;

  return (
    <div
      className="min-h-screen bg-gray-50 flex items-center justify-center px-4"
      data-testid="pay-portal"
    >
      <div className="max-w-md w-full bg-white rounded-2xl shadow-sm border border-gray-200 overflow-hidden">
        {/* Header */}
        <div className="bg-indigo-600 px-8 py-6">
          <p className="text-indigo-200 text-xs font-medium uppercase tracking-wider mb-1">
            Secure Payment
          </p>
          <p
            className="text-white text-3xl font-bold"
            data-testid="pay-amount"
          >
            {formatAmount(session.amount, session.currency)}
          </p>
        </div>

        {/* Form */}
        <div className="px-8 py-6">
          <PaymentForm
            session={session}
            tilledPublishableKey={tilledPublishableKey}
            tilledAccountId={tilledAccountId}
          />
        </div>

        {/* Footer */}
        <div className="border-t border-gray-100 px-8 py-4">
          <p className="text-xs text-gray-400 text-center">
            🔒 Secured by 7D Solutions · Payments processed by Tilled
          </p>
        </div>
      </div>
    </div>
  );
}
