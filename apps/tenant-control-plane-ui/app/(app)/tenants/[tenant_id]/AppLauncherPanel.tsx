// ============================================================
// AppLauncherPanel — Subscribed apps cards with launch buttons
// Renders one card per app from the BFF. Launch opens a new tab
// relying on the shared httpOnly cookie for auth. No tokens in URLs.
// ============================================================
'use client';

import { useQuery } from '@tanstack/react-query';
import { Button, StatusBadge } from '@/components/ui';
import { REFETCH_INTERVAL_MS } from '@/lib/constants';
import type { TenantAppListResponse, TenantApp } from '@/lib/api/types';

// ── Data fetcher ──────────────────────────────────────────

async function fetchApps(tenantId: string): Promise<TenantAppListResponse> {
  const res = await fetch(`/api/tenants/${encodeURIComponent(tenantId)}/apps`);
  if (!res.ok) throw new Error(`HTTP ${res.status}`);
  return res.json();
}

// ── Component ─────────────────────────────────────────────

interface AppLauncherPanelProps {
  tenantId: string;
}

export function AppLauncherPanel({ tenantId }: AppLauncherPanelProps) {
  const appsQuery = useQuery({
    queryKey: ['tenant', tenantId, 'apps'],
    queryFn: () => fetchApps(tenantId),
    refetchInterval: REFETCH_INTERVAL_MS,
  });

  const apps = appsQuery.data?.apps ?? [];

  function handleLaunch(app: TenantApp) {
    if (!app.launch_url) return;
    // Open in new tab — shared httpOnly cookie provides auth.
    // Never include tokens in the URL.
    window.open(app.launch_url, '_blank', 'noopener,noreferrer');
  }

  return (
    <div data-testid="app-launcher-panel">
      <h2 className="text-lg font-semibold text-[--color-text-primary] mb-4">
        Subscribed Apps
      </h2>

      {appsQuery.isLoading ? (
        <div
          className="rounded-[--radius-lg] border border-[--color-border-light] bg-[--color-bg-primary] p-8 text-center text-[--color-text-muted]"
          data-testid="apps-loading"
        >
          Loading apps...
        </div>
      ) : appsQuery.isError ? (
        <div
          className="rounded-[--radius-lg] border border-[--color-border-light] bg-[--color-bg-primary] p-8 text-center text-[--color-danger]"
          data-testid="apps-error"
        >
          Unable to load subscribed apps
        </div>
      ) : apps.length === 0 ? (
        <div
          className="rounded-[--radius-lg] border border-[--color-border-light] bg-[--color-bg-primary] p-8 text-center text-[--color-text-muted]"
          data-testid="apps-empty"
        >
          No subscribed apps for this tenant.
        </div>
      ) : (
        <div
          className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4"
          data-testid="apps-grid"
        >
          {apps.map((app) => (
            <AppCard key={app.id} app={app} onLaunch={handleLaunch} />
          ))}
        </div>
      )}
    </div>
  );
}

// ── App Card ──────────────────────────────────────────────

function AppCard({
  app,
  onLaunch,
}: {
  app: TenantApp;
  onLaunch: (app: TenantApp) => void;
}) {
  const hasLaunchUrl = !!app.launch_url;

  return (
    <div
      className="rounded-[--radius-lg] border border-[--color-border-light] bg-[--color-bg-primary] p-4 flex flex-col"
      data-testid="app-card"
    >
      <div className="flex items-center justify-between mb-2">
        <h3
          className="text-sm font-semibold text-[--color-text-primary]"
          data-testid="app-card-name"
        >
          {app.name}
        </h3>
        <StatusBadge status={app.status} variant="compact" />
      </div>

      <p className="text-xs text-[--color-text-secondary] mb-3">
        {app.module_code}
      </p>

      {!hasLaunchUrl ? (
        <p
          className="text-xs text-[--color-text-muted] mt-auto"
          data-testid="app-no-launch-url"
        >
          Launch URL not configured
        </p>
      ) : (
        <Button
          variant="primary"
          size="sm"
          className="mt-auto"
          onClick={() => onLaunch(app)}
          data-testid="app-launch-btn"
        >
          Launch
        </Button>
      )}
    </div>
  );
}
