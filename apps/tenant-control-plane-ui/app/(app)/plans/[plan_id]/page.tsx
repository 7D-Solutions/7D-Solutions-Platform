// ============================================================
// /app/plans/[plan_id] — Plan detail page
// Shows pricing rules, metered dimensions, seats, bundles,
// and entitlements via a single BFF aggregation endpoint.
// ============================================================
'use client';
import { useParams, useRouter } from 'next/navigation';
import { useQuery } from '@tanstack/react-query';
import { ArrowLeft } from 'lucide-react';
import { Button, StatusBadge } from '@/components/ui';
import { useTabActions } from '@/infrastructure/state/tabStore';
import { formatDate, formatCurrency, formatNumber } from '@/infrastructure/utils/formatters';
import { REFETCH_INTERVAL_MS } from '@/lib/constants';
import type { PlanDetail } from '@/lib/api/types';

// ── Data fetcher ────────────────────────────────────────────

async function fetchPlanDetail(planId: string): Promise<PlanDetail> {
  const res = await fetch(`/api/plans/${encodeURIComponent(planId)}`);
  if (!res.ok) throw new Error(`HTTP ${res.status}`);
  return res.json();
}

// ── Formatting helpers ──────────────────────────────────────

const PRICING_MODEL_LABELS: Record<string, string> = {
  flat: 'Flat rate',
  per_seat: 'Per seat',
  tiered: 'Tiered',
  usage: 'Usage-based',
};

function formatPricingModel(model: string): string {
  return PRICING_MODEL_LABELS[model] ?? model;
}

// ── Page component ──────────────────────────────────────────

export default function PlanDetailPage() {
  const params = useParams<{ plan_id: string }>();
  const planId = params.plan_id;
  const router = useRouter();
  const { openTab } = useTabActions();

  const { data: plan, isLoading, isError } = useQuery({
    queryKey: ['plan-detail', planId],
    queryFn: () => fetchPlanDetail(planId),
    refetchInterval: REFETCH_INTERVAL_MS,
  });

  const handleBackToList = () => {
    openTab({ id: 'plans-list', title: 'Plans & Pricing', route: '/plans', closeable: true, isPreview: false });
    router.push('/plans');
  };

  // ── Loading state ─────────────────────────────────────────
  if (isLoading) {
    return (
      <div className="py-12 text-center text-[--color-text-muted]" data-testid="plan-detail-loading">
        Loading plan details...
      </div>
    );
  }

  // ── Error state ───────────────────────────────────────────
  if (isError || !plan) {
    return (
      <div data-testid="plan-detail-error">
        <div className="rounded-[--radius-lg] border border-[--color-danger] bg-red-50 p-4 mb-4 text-sm text-[--color-danger]">
          Failed to load plan details. The plan may not exist or the service may be unavailable.
        </div>
        <Button variant="ghost" size="sm" onClick={handleBackToList} icon={ArrowLeft} iconPosition="left">
          Back to Plans
        </Button>
      </div>
    );
  }

  // ── Render plan detail ────────────────────────────────────
  return (
    <div data-testid="plan-detail">
      {/* Header */}
      <div className="flex items-center gap-3 mb-6">
        <Button variant="ghost" size="sm" onClick={handleBackToList} icon={ArrowLeft} iconPosition="left">
          Plans
        </Button>
      </div>

      <div className="flex items-start justify-between mb-6">
        <div>
          <h1 className="text-2xl font-semibold text-[--color-text-primary] mb-1" data-testid="plan-detail-name">
            {plan.name}
          </h1>
          {plan.description && (
            <p className="text-sm text-[--color-text-secondary] mb-2">{plan.description}</p>
          )}
          <div className="flex items-center gap-3 text-sm text-[--color-text-muted]">
            <span>{formatPricingModel(plan.pricing_model)}</span>
            <span>&middot;</span>
            <span>{plan.included_seats} {plan.included_seats === 1 ? 'seat' : 'seats'} included</span>
            {plan.created_at && (
              <>
                <span>&middot;</span>
                <span>Created {formatDate(plan.created_at)}</span>
              </>
            )}
          </div>
        </div>
        <StatusBadge status={plan.status} variant="large" />
      </div>

      {/* Sections grid */}
      <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
        {/* Pricing Rules */}
        <DetailSection title="Pricing Rules" testId="plan-pricing-rules">
          {plan.pricing_rules.length === 0 ? (
            <EmptyHint>No pricing rules defined.</EmptyHint>
          ) : (
            <div className="space-y-2">
              {plan.pricing_rules.map((rule) => (
                <div
                  key={rule.id}
                  className="flex items-center justify-between rounded-[--radius-default] border border-[--color-border-light] bg-[--color-bg-secondary] px-3 py-2"
                >
                  <div>
                    <p className="text-sm font-medium text-[--color-text-primary]">{rule.label}</p>
                    <p className="text-xs text-[--color-text-muted]">
                      {rule.type}
                      {rule.per_unit ? ` / ${rule.per_unit}` : ''}
                      {rule.tier_min != null && ` (${formatNumber(rule.tier_min)}–${rule.tier_max != null ? formatNumber(rule.tier_max) : '∞'})`}
                    </p>
                  </div>
                  <span className="text-sm font-semibold text-[--color-text-primary]">
                    {formatCurrency(rule.amount, rule.currency ?? 'USD')}
                  </span>
                </div>
              ))}
            </div>
          )}
        </DetailSection>

        {/* Metered Dimensions */}
        <DetailSection title="Metered Dimensions" testId="plan-metered-dimensions">
          {plan.metered_dimensions.length === 0 ? (
            <EmptyHint>No metered dimensions.</EmptyHint>
          ) : (
            <div className="space-y-2">
              {plan.metered_dimensions.map((dim) => (
                <div
                  key={dim.key}
                  className="rounded-[--radius-default] border border-[--color-border-light] bg-[--color-bg-secondary] px-3 py-2"
                >
                  <div className="flex items-center justify-between">
                    <p className="text-sm font-medium text-[--color-text-primary]">{dim.label}</p>
                    <span className="text-xs text-[--color-text-muted]">{dim.unit}</span>
                  </div>
                  <div className="flex gap-4 mt-1 text-xs text-[--color-text-secondary]">
                    {dim.included_quota != null && (
                      <span>Included: {formatNumber(dim.included_quota)}</span>
                    )}
                    {dim.overage_rate != null && (
                      <span>Overage: {formatCurrency(dim.overage_rate)}/{dim.unit}</span>
                    )}
                  </div>
                </div>
              ))}
            </div>
          )}
        </DetailSection>

        {/* Associated Bundles */}
        <DetailSection title="Associated Bundles" testId="plan-bundles">
          {plan.bundles.length === 0 ? (
            <EmptyHint>No bundles associated.</EmptyHint>
          ) : (
            <div className="space-y-2">
              {plan.bundles.map((b) => (
                <div
                  key={b.id}
                  className="flex items-center justify-between rounded-[--radius-default] border border-[--color-border-light] bg-[--color-bg-secondary] px-3 py-2"
                >
                  <span className="text-sm text-[--color-text-primary]">{b.name}</span>
                  <StatusBadge status={b.status} variant="compact" />
                </div>
              ))}
            </div>
          )}
        </DetailSection>

        {/* Entitlements */}
        <DetailSection title="Entitlements" testId="plan-entitlements">
          {plan.entitlements.length === 0 ? (
            <EmptyHint>No entitlements defined.</EmptyHint>
          ) : (
            <div className="space-y-2">
              {plan.entitlements.map((ent) => (
                <div
                  key={ent.id}
                  className="flex items-center justify-between rounded-[--radius-default] border border-[--color-border-light] bg-[--color-bg-secondary] px-3 py-2"
                >
                  <div>
                    <p className="text-sm font-medium text-[--color-text-primary]">{ent.label}</p>
                    <p className="text-xs text-[--color-text-muted]">{ent.key}</p>
                  </div>
                  <span className="text-sm font-semibold text-[--color-text-primary]">
                    {String(ent.value)}
                  </span>
                </div>
              ))}
            </div>
          )}
        </DetailSection>
      </div>
    </div>
  );
}

// ── Shared sub-components ───────────────────────────────────

function DetailSection({
  title,
  testId,
  children,
}: {
  title: string;
  testId: string;
  children: React.ReactNode;
}) {
  return (
    <section data-testid={testId}>
      <h2 className="text-lg font-semibold text-[--color-text-primary] mb-3">{title}</h2>
      {children}
    </section>
  );
}

function EmptyHint({ children }: { children: React.ReactNode }) {
  return (
    <p className="text-sm text-[--color-text-muted] italic">{children}</p>
  );
}
