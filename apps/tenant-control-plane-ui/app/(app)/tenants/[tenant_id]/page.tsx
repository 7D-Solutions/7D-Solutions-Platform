// ============================================================
// /app/tenants/[tenant_id] — Tenant Detail page
// Overview tab: status, plan summary, health snapshot, key dates.
// Future tabs (Billing, Access, Features, Settings, Activity)
// will be added by downstream beads.
// ============================================================
'use client';
import { useParams } from 'next/navigation';
import { useQuery } from '@tanstack/react-query';
import Link from 'next/link';
import { ArrowLeft } from 'lucide-react';
import { Button, StatusBadge } from '@/components/ui';
import { useViewStore } from '@/infrastructure/state/useViewStore';
import { formatDate } from '@/infrastructure/utils/formatters';
import { REFETCH_INTERVAL_MS } from '@/lib/constants';
import type { TenantDetail, TenantPlanSummary, HealthSnapshot } from '@/lib/api/types';

// ── Data fetchers ──────────────────────────────────────────

async function fetchTenantDetail(tenantId: string): Promise<TenantDetail> {
  const res = await fetch(`/api/tenants/${encodeURIComponent(tenantId)}`);
  if (!res.ok) throw new Error(`HTTP ${res.status}`);
  return res.json();
}

async function fetchPlanSummary(tenantId: string): Promise<TenantPlanSummary> {
  const res = await fetch(`/api/tenants/${encodeURIComponent(tenantId)}/plan-summary`);
  if (!res.ok) throw new Error(`HTTP ${res.status}`);
  return res.json();
}

async function fetchHealthSnapshot(): Promise<HealthSnapshot> {
  const res = await fetch('/api/system/health-snapshot');
  if (!res.ok) throw new Error(`HTTP ${res.status}`);
  return res.json();
}

// ── Tab definitions ────────────────────────────────────────

const TABS = [
  { id: 'overview',  label: 'Overview' },
  { id: 'billing',   label: 'Billing' },
  { id: 'access',    label: 'Access' },
  { id: 'features',  label: 'Features' },
  { id: 'settings',  label: 'Settings' },
  { id: 'activity',  label: 'Activity' },
] as const;

// ── Page component ─────────────────────────────────────────

export default function TenantDetailPage() {
  const { tenant_id } = useParams<{ tenant_id: string }>();
  const { state, setState } = useViewStore('tenant-detail', { activeTab: 0 });
  const activeTabIndex = state.activeTab as number;

  // Tenant detail — tenant-scoped query key
  const tenantQuery = useQuery({
    queryKey: ['tenant', tenant_id, 'detail'],
    queryFn: () => fetchTenantDetail(tenant_id),
    refetchInterval: REFETCH_INTERVAL_MS,
  });

  // Plan summary — tenant-scoped query key
  const planQuery = useQuery({
    queryKey: ['tenant', tenant_id, 'plan-summary'],
    queryFn: () => fetchPlanSummary(tenant_id),
    enabled: activeTabIndex === 0,
  });

  // Health snapshot — not tenant-scoped (platform-wide)
  const healthQuery = useQuery({
    queryKey: ['system', 'health-snapshot'],
    queryFn: fetchHealthSnapshot,
    refetchInterval: REFETCH_INTERVAL_MS,
    enabled: activeTabIndex === 0,
  });

  const tenant = tenantQuery.data;
  const activeTab = TABS[activeTabIndex];

  return (
    <div>
      {/* Back link + header */}
      <div className="mb-4">
        <Link
          href="/tenants"
          className="inline-flex items-center gap-1 text-sm text-[--color-text-secondary] hover:text-[--color-primary] mb-2"
        >
          <ArrowLeft className="h-3.5 w-3.5" />
          Back to Tenants
        </Link>

        {tenantQuery.isLoading ? (
          <div className="h-8 w-48 bg-[--color-bg-secondary] rounded animate-pulse" />
        ) : (
          <div className="flex items-center gap-3">
            <h1 className="text-2xl font-semibold text-[--color-text-primary]">
              {tenant?.name ?? 'Tenant'}
            </h1>
            {tenant && <StatusBadge status={tenant.status} />}
          </div>
        )}
      </div>

      {/* Tab bar */}
      <div
        className="flex border-b border-[--color-border-light] mb-6"
        role="tablist"
        data-testid="tenant-detail-tabs"
      >
        {TABS.map((tab, index) => (
          <button
            key={tab.id}
            role="tab"
            aria-selected={activeTabIndex === index}
            onClick={() => setState({ activeTab: index })}
            className={`px-4 py-2.5 text-sm font-medium border-b-2 transition-[--transition-fast] ${
              activeTabIndex === index
                ? 'border-[--color-primary] text-[--color-primary]'
                : 'border-transparent text-[--color-text-secondary] hover:text-[--color-text-primary] hover:border-[--color-border-default]'
            }`}
            data-testid={`tab-${tab.id}`}
          >
            {tab.label}
          </button>
        ))}
      </div>

      {/* Tab content */}
      {activeTab?.id === 'overview' && (
        <OverviewTab
          tenant={tenant}
          tenantLoading={tenantQuery.isLoading}
          tenantError={tenantQuery.isError}
          plan={planQuery.data}
          planLoading={planQuery.isLoading}
          planError={planQuery.isError}
          health={healthQuery.data}
          healthLoading={healthQuery.isLoading}
          healthError={healthQuery.isError}
        />
      )}

      {activeTab && activeTab.id !== 'overview' && (
        <div
          className="rounded-[--radius-lg] border border-[--color-border-light] bg-[--color-bg-primary] p-8 text-center text-[--color-text-muted]"
          data-testid="tab-placeholder"
        >
          {activeTab.label} tab — coming soon
        </div>
      )}
    </div>
  );
}

// ── Overview Tab ───────────────────────────────────────────

function OverviewTab({
  tenant,
  tenantLoading,
  tenantError,
  plan,
  planLoading,
  planError,
  health,
  healthLoading,
  healthError,
}: {
  tenant: TenantDetail | undefined;
  tenantLoading: boolean;
  tenantError: boolean;
  plan: TenantPlanSummary | undefined;
  planLoading: boolean;
  planError: boolean;
  health: HealthSnapshot | undefined;
  healthLoading: boolean;
  healthError: boolean;
}) {
  return (
    <div className="grid grid-cols-1 lg:grid-cols-2 gap-4" data-testid="overview-tab">
      {/* Status & Details card */}
      <Card title="Status & Details" testId="status-card">
        {tenantLoading ? (
          <LoadingSkeleton rows={4} />
        ) : tenantError ? (
          <ErrorMessage message="Unable to load tenant details" />
        ) : tenant ? (
          <dl className="space-y-3">
            <DetailRow label="Status">
              <StatusBadge status={tenant.status} />
            </DetailRow>
            <DetailRow label="Plan">{tenant.plan}</DetailRow>
            {tenant.app_id && (
              <DetailRow label="Connection ID">{tenant.app_id}</DetailRow>
            )}
            {tenant.user_count !== undefined && (
              <DetailRow label="Users">
                {tenant.user_count}
                {tenant.seat_limit ? ` / ${tenant.seat_limit} seats` : ''}
              </DetailRow>
            )}
          </dl>
        ) : null}
      </Card>

      {/* Plan Summary card */}
      <Card title="Plan Summary" testId="plan-summary-card">
        {planLoading ? (
          <LoadingSkeleton rows={3} />
        ) : planError ? (
          <ErrorMessage message="Unable to load plan information" />
        ) : plan ? (
          <dl className="space-y-3">
            <DetailRow label="Plan">{plan.plan_name}</DetailRow>
            <DetailRow label="Pricing Model">{plan.pricing_model}</DetailRow>
            <DetailRow label="Included Seats">{plan.included_seats}</DetailRow>
            {plan.metered_dimensions.length > 0 && (
              <DetailRow label="Metered Dimensions">
                {plan.metered_dimensions.join(', ')}
              </DetailRow>
            )}
            {plan.assigned_at && (
              <DetailRow label="Assigned">{formatDate(plan.assigned_at)}</DetailRow>
            )}
          </dl>
        ) : null}
      </Card>

      {/* Key Dates card */}
      <Card title="Key Dates" testId="key-dates-card">
        {tenantLoading ? (
          <LoadingSkeleton rows={3} />
        ) : tenantError ? (
          <ErrorMessage message="Unable to load dates" />
        ) : tenant ? (
          <dl className="space-y-3">
            <DetailRow label="Created">{formatDate(tenant.created_at)}</DetailRow>
            {tenant.activated_at && (
              <DetailRow label="Activated">{formatDate(tenant.activated_at)}</DetailRow>
            )}
            {tenant.suspended_at && (
              <DetailRow label="Suspended">{formatDate(tenant.suspended_at)}</DetailRow>
            )}
            {tenant.terminated_at && (
              <DetailRow label="Terminated">{formatDate(tenant.terminated_at)}</DetailRow>
            )}
            <DetailRow label="Last Updated">{formatDate(tenant.updated_at)}</DetailRow>
          </dl>
        ) : null}
      </Card>

      {/* Health Snapshot card */}
      <Card title="Platform Health" testId="health-snapshot-card">
        {healthLoading ? (
          <LoadingSkeleton rows={4} />
        ) : healthError ? (
          <ErrorMessage message="Unable to check service health" />
        ) : health ? (
          <div className="space-y-2">
            {health.services.map((svc) => (
              <div
                key={svc.service}
                className="flex items-center justify-between py-1"
                data-testid="health-service-row"
              >
                <span className="text-sm text-[--color-text-primary]">{svc.service}</span>
                <div className="flex items-center gap-2">
                  <StatusBadge status={svc.status} variant="compact" />
                  {svc.latency_ms !== undefined && (
                    <span className="text-xs text-[--color-text-muted]">
                      {svc.latency_ms}ms
                    </span>
                  )}
                </div>
              </div>
            ))}
            <p className="text-xs text-[--color-text-muted] pt-2">
              Checked {formatDate(health.checked_at)}
            </p>
          </div>
        ) : null}
      </Card>
    </div>
  );
}

// ── Shared sub-components ──────────────────────────────────

function Card({
  title,
  testId,
  children,
}: {
  title: string;
  testId: string;
  children: React.ReactNode;
}) {
  return (
    <div
      className="rounded-[--radius-lg] border border-[--color-border-light] bg-[--color-bg-primary] p-5"
      data-testid={testId}
    >
      <h2 className="text-sm font-semibold text-[--color-text-primary] mb-3 pb-2 border-b border-[--color-border-light]">
        {title}
      </h2>
      {children}
    </div>
  );
}

function DetailRow({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div className="flex items-start justify-between">
      <dt className="text-sm text-[--color-text-secondary]">{label}</dt>
      <dd className="text-sm font-medium text-[--color-text-primary] text-right">
        {children}
      </dd>
    </div>
  );
}

function LoadingSkeleton({ rows }: { rows: number }) {
  return (
    <div className="space-y-3">
      {Array.from({ length: rows }, (_, i) => (
        <div key={i} className="flex justify-between">
          <div className="h-4 w-20 bg-[--color-bg-secondary] rounded animate-pulse" />
          <div className="h-4 w-28 bg-[--color-bg-secondary] rounded animate-pulse" />
        </div>
      ))}
    </div>
  );
}

function ErrorMessage({ message }: { message: string }) {
  return (
    <div className="text-sm text-[--color-danger] py-2" data-testid="error-message">
      {message}
    </div>
  );
}
