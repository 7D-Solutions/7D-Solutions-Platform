// ============================================================
// /app/billing — Billing overview (placeholder)
// ============================================================
import { CreditCard } from 'lucide-react';

export default function BillingPage() {
  return (
    <div data-testid="billing-page">
      <div className="mb-6">
        <h1 className="text-2xl font-bold text-[--color-text-primary]">Billing</h1>
        <p className="text-sm text-[--color-text-secondary] mt-1">
          Subscription billing, invoices, and payment history
        </p>
      </div>

      <div className="rounded-lg border border-[--color-border-light] bg-[--color-bg-primary] p-8 text-center">
        <CreditCard className="h-12 w-12 text-[--color-text-muted] mx-auto mb-4" />
        <h2 className="text-lg font-semibold text-[--color-text-primary] mb-2">
          Billing Overview
        </h2>
        <p className="text-sm text-[--color-text-secondary] max-w-md mx-auto mb-6">
          View and manage tenant billing cycles, outstanding invoices, payment
          methods, and revenue summaries across all active subscriptions.
        </p>
        <div className="grid grid-cols-1 sm:grid-cols-3 gap-4 max-w-lg mx-auto">
          <PlannedFeature title="Invoices" description="Browse and search all generated invoices" />
          <PlannedFeature title="Payments" description="Track payment status and retry failures" />
          <PlannedFeature title="Revenue" description="Monthly recurring revenue dashboard" />
        </div>
      </div>
    </div>
  );
}

function PlannedFeature({ title, description }: { title: string; description: string }) {
  return (
    <div className="rounded-[--radius-default] border border-dashed border-[--color-border-default] p-3">
      <p className="text-sm font-medium text-[--color-text-primary]">{title}</p>
      <p className="text-xs text-[--color-text-muted] mt-0.5">{description}</p>
    </div>
  );
}
