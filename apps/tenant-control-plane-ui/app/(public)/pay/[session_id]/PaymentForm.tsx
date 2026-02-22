// ============================================================
// PaymentForm — client component for hosted pay portal
// Renders Tilled.js checkout form when publishable key is present,
// or a mock form in dev/test environments.
//
// Security invariants:
//   - client_secret NEVER appears in any URL or query param
//   - return_url / cancel_url used directly from server response
//   - No dynamic script construction from user data
// ============================================================
'use client';

import { useEffect, useRef, useState, useCallback } from 'react';
import { Button } from '@/components/ui/Button';

export interface SessionData {
  session_id: string;
  status: string;
  amount: number;
  currency: string;
  client_secret: string;
  return_url: string | null;
  cancel_url: string | null;
}

interface PaymentFormProps {
  session: SessionData;
  tilledPublishableKey: string | null;
  tilledAccountId: string | null;
}

type PaymentState = 'idle' | 'loading' | 'ready' | 'processing' | 'success' | 'error';

function formatAmount(amount: number, currency: string): string {
  return new Intl.NumberFormat('en-US', {
    style: 'currency',
    currency: currency.toUpperCase(),
  }).format(amount / 100);
}

// ── Real Tilled.js form ─────────────────────────────────────

function TilledPaymentForm({
  session,
  publishableKey,
  accountId,
}: {
  session: SessionData;
  publishableKey: string;
  accountId: string;
}) {
  const [state, setState] = useState<PaymentState>('loading');
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const tilledRef = useRef<unknown>(null);
  const formRef = useRef<unknown>(null);

  const handleSuccess = useCallback(() => {
    setState('success');
    if (session.return_url) {
      // Small delay to let the user see the success state
      setTimeout(() => {
        window.location.href = session.return_url!;
      }, 1500);
    }
  }, [session.return_url]);

  const handleCancel = useCallback(() => {
    if (session.cancel_url) {
      window.location.href = session.cancel_url;
    }
  }, [session.cancel_url]);

  useEffect(() => {
    let cancelled = false;

    async function initTilled() {
      // Dynamically load Tilled.js from CDN
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      if (!(window as any)['Tilled']) {
        await new Promise<void>((resolve, reject) => {
          const script = document.createElement('script');
          script.src = 'https://js.tilled.com/v2';
          script.onload = () => resolve();
          script.onerror = () => reject(new Error('Failed to load Tilled.js'));
          document.head.appendChild(script);
        });
      }

      if (cancelled) return;

      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      const TilledCtor = (window as any)['Tilled'] as new (
        key: string,
        accountId: string,
        opts: { sandbox: boolean; log_level: number },
      ) => unknown;

      const tilled = new TilledCtor(publishableKey, accountId, {
        sandbox: !publishableKey.startsWith('pk_live_'),
        log_level: 0,
      });
      tilledRef.current = tilled;

      const tilledApi = tilled as {
        form: (opts: { payment_method_type: string }) => Promise<{
          createField: (type: string) => { inject: (selector: string) => void };
          build: () => Promise<void>;
          paymentMethod: unknown;
        }>;
        confirmPayment: (
          secret: string,
          opts: { payment_method: unknown },
        ) => Promise<{ error?: { message: string } }>;
      };

      const form = await tilledApi.form({ payment_method_type: 'card' });
      formRef.current = form;

      form.createField('cardNumber').inject('#tilled-card-number');
      form.createField('cardExpiry').inject('#tilled-card-expiry');
      form.createField('cardCvv').inject('#tilled-card-cvv');

      await form.build();

      if (!cancelled) {
        setState('ready');
      }

      return { tilled: tilledApi, form };
    }

    initTilled().catch((err: unknown) => {
      if (!cancelled) {
        const msg = err instanceof Error ? err.message : 'Failed to initialize payment form';
        setErrorMsg(msg);
        setState('error');
      }
    });

    return () => {
      cancelled = true;
    };
  }, [publishableKey, accountId]);

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (state !== 'ready' || !tilledRef.current || !formRef.current) return;

    setState('processing');
    setErrorMsg(null);

    const tilledApi = tilledRef.current as {
      confirmPayment: (
        secret: string,
        opts: { payment_method: unknown },
      ) => Promise<{ error?: { message: string } }>;
    };
    const form = formRef.current as { paymentMethod: unknown };

    const { error } = await tilledApi.confirmPayment(session.client_secret, {
      payment_method: form.paymentMethod,
    });

    if (error) {
      setErrorMsg(error.message ?? 'Payment failed. Please try again.');
      setState('ready');
      return;
    }

    handleSuccess();
  }

  if (state === 'success') {
    return (
      <div data-testid="pay-success" className="text-center py-8">
        <div className="text-green-600 text-5xl mb-4">✓</div>
        <p className="text-lg font-semibold text-gray-900">Payment successful!</p>
        {session.return_url && (
          <p className="text-sm text-gray-500 mt-2">Redirecting you back…</p>
        )}
      </div>
    );
  }

  return (
    <form onSubmit={handleSubmit} data-testid="tilled-payment-form">
      <div className="space-y-4">
        <div>
          <label className="block text-sm font-medium text-gray-700 mb-1">Card Number</label>
          <div
            id="tilled-card-number"
            className="border border-gray-300 rounded-md p-3 min-h-[42px] bg-white"
            data-testid="tilled-card-number"
          />
        </div>
        <div className="grid grid-cols-2 gap-4">
          <div>
            <label className="block text-sm font-medium text-gray-700 mb-1">Expiry</label>
            <div
              id="tilled-card-expiry"
              className="border border-gray-300 rounded-md p-3 min-h-[42px] bg-white"
              data-testid="tilled-card-expiry"
            />
          </div>
          <div>
            <label className="block text-sm font-medium text-gray-700 mb-1">CVV</label>
            <div
              id="tilled-card-cvv"
              className="border border-gray-300 rounded-md p-3 min-h-[42px] bg-white"
              data-testid="tilled-card-cvv"
            />
          </div>
        </div>
      </div>

      {errorMsg && (
        <div
          data-testid="pay-error"
          className="mt-4 rounded-md bg-red-50 border border-red-200 px-4 py-3 text-sm text-red-700"
        >
          {errorMsg}
        </div>
      )}

      <div className="mt-6 flex flex-col gap-3">
        <Button
          type="submit"
          variant="primary"
          size="lg"
          disabled={state !== 'ready'}
          loading={state === 'processing'}
          data-testid="pay-submit"
          className="w-full"
        >
          {state === 'loading' && 'Loading…'}
          {(state === 'ready' || state === 'processing') &&
            `Pay ${formatAmount(session.amount, session.currency)}`}
        </Button>

        {session.cancel_url && (
          <Button
            type="button"
            variant="outline"
            size="lg"
            onClick={handleCancel}
            data-testid="pay-cancel"
            className="w-full"
          >
            Cancel
          </Button>
        )}
      </div>
    </form>
  );
}

// ── Mock form (dev/test — no Tilled credentials) ────────────

function MockPaymentForm({ session }: { session: SessionData }) {
  const [state, setState] = useState<'idle' | 'processing' | 'success'>('idle');

  function handleSimulateSuccess() {
    setState('processing');
    setTimeout(() => {
      setState('success');
      if (session.return_url) {
        setTimeout(() => {
          window.location.href = session.return_url!;
        }, 1000);
      }
    }, 800);
  }

  function handleCancel() {
    if (session.cancel_url) {
      window.location.href = session.cancel_url;
    }
  }

  if (state === 'success') {
    return (
      <div data-testid="pay-success" className="text-center py-8">
        <div className="text-green-600 text-5xl mb-4">✓</div>
        <p className="text-lg font-semibold text-gray-900">Payment simulated!</p>
        {session.return_url && (
          <p className="text-sm text-gray-500 mt-2">Redirecting…</p>
        )}
      </div>
    );
  }

  return (
    <div data-testid="mock-payment-form">
      <div className="mb-4 rounded-md bg-amber-50 border border-amber-200 px-4 py-3">
        <p className="text-sm text-amber-700 font-medium">
          ⚠ Mock payment mode — no real charges
        </p>
      </div>

      <div className="space-y-3">
        <div>
          <label className="block text-sm font-medium text-gray-700 mb-1">Card Number</label>
          <input
            type="text"
            defaultValue="4111 1111 1111 1111"
            readOnly
            className="w-full border border-gray-300 rounded-md px-3 py-2 bg-gray-50 text-gray-500 text-sm"
            data-testid="mock-card-number"
          />
        </div>
        <div className="grid grid-cols-2 gap-3">
          <div>
            <label className="block text-sm font-medium text-gray-700 mb-1">Expiry</label>
            <input
              type="text"
              defaultValue="12/29"
              readOnly
              className="w-full border border-gray-300 rounded-md px-3 py-2 bg-gray-50 text-gray-500 text-sm"
            />
          </div>
          <div>
            <label className="block text-sm font-medium text-gray-700 mb-1">CVV</label>
            <input
              type="text"
              defaultValue="123"
              readOnly
              className="w-full border border-gray-300 rounded-md px-3 py-2 bg-gray-50 text-gray-500 text-sm"
            />
          </div>
        </div>
      </div>

      <div className="mt-6 flex flex-col gap-3">
        <Button
          variant="primary"
          size="lg"
          onClick={handleSimulateSuccess}
          loading={state === 'processing'}
          disabled={state === 'processing'}
          data-testid="pay-submit"
          className="w-full"
        >
          {`Pay ${formatAmount(session.amount, session.currency)}`}
        </Button>

        {session.cancel_url && (
          <Button
            type="button"
            variant="outline"
            size="lg"
            onClick={handleCancel}
            data-testid="pay-cancel"
            className="w-full"
          >
            Cancel
          </Button>
        )}
      </div>
    </div>
  );
}

// ── Main export ─────────────────────────────────────────────

export function PaymentForm({
  session,
  tilledPublishableKey,
  tilledAccountId,
}: PaymentFormProps) {
  const hasTilled = !!(tilledPublishableKey && tilledAccountId);

  return (
    <>
      {hasTilled ? (
        <TilledPaymentForm
          session={session}
          publishableKey={tilledPublishableKey}
          accountId={tilledAccountId}
        />
      ) : (
        <MockPaymentForm session={session} />
      )}
    </>
  );
}
