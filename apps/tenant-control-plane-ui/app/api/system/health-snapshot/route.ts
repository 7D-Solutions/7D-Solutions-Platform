// ============================================================
// GET /api/system/health-snapshot — BFF aggregator for service health
// Calls readiness endpoints on backend services with timeouts.
// Each service is checked independently — partial failures are reported,
// not propagated. The UI renders a degraded badge for unhealthy services.
// Auth: requires platform_admin JWT in httpOnly cookie
// ============================================================
import { NextResponse } from 'next/server';
import { guardPlatformAdmin } from '@/lib/server/auth';
import {
  TENANT_REGISTRY_BASE_URL,
  TTP_BASE_URL,
  AR_BASE_URL,
  IDENTITY_AUTH_BASE_URL,
} from '@/lib/constants';
import type { ServiceHealth, HealthSnapshot } from '@/lib/api/types';

const SERVICES = [
  { name: 'Tenant Registry', url: `${TENANT_REGISTRY_BASE_URL}/health` },
  { name: 'Plans & Pricing', url: `${TTP_BASE_URL}/health` },
  { name: 'Billing',         url: `${AR_BASE_URL}/health` },
  { name: 'Identity & Auth', url: `${IDENTITY_AUTH_BASE_URL}/health` },
];

const HEALTH_TIMEOUT_MS = 3000;

async function checkService(
  name: string,
  url: string,
): Promise<ServiceHealth> {
  const start = Date.now();
  try {
    const res = await fetch(url, {
      signal: AbortSignal.timeout(HEALTH_TIMEOUT_MS),
    });
    const latency_ms = Date.now() - start;

    if (res.ok) {
      return { service: name, status: 'available', latency_ms };
    }
    return { service: name, status: 'degraded', latency_ms };
  } catch {
    return { service: name, status: 'unavailable', latency_ms: Date.now() - start };
  }
}

export async function GET() {
  const auth = await guardPlatformAdmin();
  if (auth instanceof Response) return auth;

  const services = await Promise.all(
    SERVICES.map((s) => checkService(s.name, s.url)),
  );

  const snapshot: HealthSnapshot = {
    services,
    checked_at: new Date().toISOString(),
  };

  return NextResponse.json(snapshot);
}
