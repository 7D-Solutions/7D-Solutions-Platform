// ============================================================
// /app/system/status — Platform health dashboard
// Shows service readiness tiles with polling. Partial failures
// render degraded badges — the page never blocks on one service.
// ============================================================
'use client';
import { useQuery } from '@tanstack/react-query';
import { Activity, RefreshCw } from 'lucide-react';
import { StatusBadge } from '@/components/ui';
import { Button } from '@/components/ui/Button';
import { REFETCH_INTERVAL_MS } from '@/lib/constants';
import type { HealthSnapshot, ServiceHealth } from '@/lib/api/types';

// ── Data fetcher ───────────────────────────────────────────

async function fetchHealthSnapshot(): Promise<HealthSnapshot> {
  const res = await fetch('/api/system/health-snapshot');
  if (!res.ok) throw new Error(`HTTP ${res.status}`);
  return res.json();
}

// ── Helpers ────────────────────────────────────────────────

function formatCheckedAt(iso: string | undefined): string {
  if (!iso) return '—';
  try {
    return new Date(iso).toLocaleString('en-US', {
      month: 'short',
      day: 'numeric',
      hour: 'numeric',
      minute: '2-digit',
      second: '2-digit',
    });
  } catch {
    return iso;
  }
}

function overallStatus(services: ServiceHealth[]): string {
  if (services.every((s) => s.status === 'available')) return 'All systems operational';
  if (services.some((s) => s.status === 'unavailable')) return 'Service outage detected';
  return 'Degraded performance';
}

function overallStatusColor(services: ServiceHealth[]): string {
  if (services.every((s) => s.status === 'available')) return 'text-green-600';
  if (services.some((s) => s.status === 'unavailable')) return 'text-red-600';
  return 'text-yellow-600';
}

// ── Service Tile ───────────────────────────────────────────

function ServiceTile({ service }: { service: ServiceHealth }) {
  return (
    <div
      className="rounded-lg border border-[--color-border-light] bg-[--color-bg-primary] p-4 flex items-center justify-between"
      data-testid={`service-tile-${service.service.toLowerCase().replace(/[^a-z0-9]+/g, '-')}`}
    >
      <div className="flex items-center gap-3">
        <Activity className="h-5 w-5 text-[--color-text-secondary]" />
        <span className="text-sm font-medium text-[--color-text-primary]">
          {service.service}
        </span>
      </div>
      <div className="flex items-center gap-3">
        {service.latency_ms != null && (
          <span className="text-xs text-[--color-text-muted]">
            {service.latency_ms}ms
          </span>
        )}
        <StatusBadge status={service.status} />
      </div>
    </div>
  );
}

// ── Page ───────────────────────────────────────────────────

export default function SystemStatusPage() {
  const { data, isLoading, isError, refetch, isFetching, dataUpdatedAt } = useQuery({
    queryKey: ['system', 'health-snapshot'],
    queryFn: fetchHealthSnapshot,
    refetchInterval: REFETCH_INTERVAL_MS,
  });

  return (
    <div data-testid="system-status-page">
      {/* Header */}
      <div className="flex items-center justify-between mb-6">
        <div>
          <h1 className="text-2xl font-bold text-[--color-text-primary]">System Status</h1>
          <p className="text-sm text-[--color-text-secondary] mt-1">
            Platform service health and readiness
          </p>
        </div>
        <div className="flex items-center gap-3">
          {dataUpdatedAt > 0 && (
            <span className="text-xs text-[--color-text-muted]" data-testid="last-updated">
              Updated {formatCheckedAt(data?.checked_at)}
            </span>
          )}
          <Button
            variant="outline"
            size="sm"
            onClick={() => refetch()}
            disabled={isFetching}
            icon={RefreshCw}
            iconPosition="left"
            data-testid="refresh-btn"
          >
            Refresh
          </Button>
        </div>
      </div>

      {/* Overall status banner */}
      {data && (
        <div
          className="rounded-lg border border-[--color-border-light] bg-[--color-bg-primary] p-4 mb-6 flex items-center gap-3"
          data-testid="overall-status"
        >
          <Activity className="h-5 w-5" />
          <span className={`text-sm font-semibold ${overallStatusColor(data.services)}`}>
            {overallStatus(data.services)}
          </span>
        </div>
      )}

      {/* Loading state */}
      {isLoading && (
        <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
          {[1, 2, 3, 4].map((i) => (
            <div
              key={i}
              className="rounded-lg border border-[--color-border-light] bg-[--color-bg-primary] p-4 h-16 animate-pulse"
            />
          ))}
        </div>
      )}

      {/* Error state */}
      {isError && !data && (
        <div
          className="rounded-lg border border-red-200 bg-red-50 p-4 text-sm text-red-700"
          data-testid="status-error"
        >
          Unable to check service health. Backend may be unreachable.
        </div>
      )}

      {/* Service tiles */}
      {data && (
        <div className="grid grid-cols-1 md:grid-cols-2 gap-4" data-testid="service-tiles">
          {data.services.map((svc) => (
            <ServiceTile key={svc.service} service={svc} />
          ))}
        </div>
      )}
    </div>
  );
}
