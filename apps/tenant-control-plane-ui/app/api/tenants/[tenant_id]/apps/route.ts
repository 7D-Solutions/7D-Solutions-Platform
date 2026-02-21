// ============================================================
// GET /api/tenants/[tenant_id]/apps — BFF: subscribed apps
// Returns the list of apps available to a tenant (from bundle).
// Falls back to seed data when tenant-registry is unavailable.
// Auth: requires platform_admin JWT in httpOnly cookie
// ============================================================
import { NextRequest, NextResponse } from 'next/server';
import { guardPlatformAdmin } from '@/lib/server/auth';
import { TENANT_REGISTRY_BASE_URL } from '@/lib/constants';
import { TenantAppListResponseSchema } from '@/lib/api/types';
import type { TenantAppListResponse } from '@/lib/api/types';

export async function GET(
  _request: NextRequest,
  { params }: { params: Promise<{ tenant_id: string }> }
) {
  const auth = await guardPlatformAdmin();
  if (auth instanceof Response) return auth;

  const { tenant_id } = await params;

  // Try to fetch the tenant's bundle modules from tenant-registry summary
  try {
    const upstreamUrl = `${TENANT_REGISTRY_BASE_URL}/api/control/tenants/${encodeURIComponent(tenant_id)}/summary`;
    const res = await fetch(upstreamUrl, {
      headers: { 'Content-Type': 'application/json' },
      signal: AbortSignal.timeout(5000),
    });

    if (res.ok) {
      const summary = await res.json();
      // Derive apps from the summary's module readiness data
      if (summary.modules && Array.isArray(summary.modules)) {
        const apps = summary.modules.map((mod: { module: string; status: string }) => ({
          id: `mod-${mod.module}`,
          name: formatModuleName(mod.module),
          module_code: mod.module,
          launch_url: moduleLaunchUrl(mod.module),
          status: mod.status === 'ready' ? 'available' : 'unavailable',
        }));
        const response: TenantAppListResponse = { apps };
        const parsed = TenantAppListResponseSchema.safeParse(response);
        if (parsed.success) {
          return NextResponse.json(parsed.data);
        }
        return NextResponse.json(response);
      }
    }
    // Non-ok or no modules — fall through to seed data
  } catch {
    // tenant-registry unavailable — fall through to seed data
  }

  // Seed data: known platform modules
  const fallback: TenantAppListResponse = {
    apps: [
      { id: 'mod-ar', name: 'Accounts Receivable', module_code: 'ar', launch_url: null, status: 'available' },
      { id: 'mod-gl', name: 'General Ledger', module_code: 'gl', launch_url: null, status: 'available' },
      { id: 'mod-payments', name: 'Payments', module_code: 'payments', launch_url: null, status: 'available' },
      { id: 'mod-subscriptions', name: 'Subscriptions', module_code: 'subscriptions', launch_url: null, status: 'available' },
      { id: 'mod-notifications', name: 'Notifications', module_code: 'notifications', launch_url: null, status: 'available' },
    ],
  };
  return NextResponse.json(fallback);
}

function formatModuleName(code: string): string {
  const names: Record<string, string> = {
    ar: 'Accounts Receivable',
    gl: 'General Ledger',
    payments: 'Payments',
    subscriptions: 'Subscriptions',
    notifications: 'Notifications',
    inventory: 'Inventory',
  };
  return names[code] ?? code.charAt(0).toUpperCase() + code.slice(1);
}

function moduleLaunchUrl(code: string): string | null {
  // Launch URLs will be configured per-deployment; null until app frontends exist
  const urls: Record<string, string> = {};
  return urls[code] ?? null;
}
