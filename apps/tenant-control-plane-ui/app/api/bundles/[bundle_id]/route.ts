// ============================================================
// GET /api/bundles/[bundle_id] — BFF proxy to TTP bundle detail
// Returns full composition (entitlements list).
// Auth: requires platform_admin JWT in httpOnly cookie
// Falls back to seed data when TTP is unavailable.
// ============================================================
import { NextRequest, NextResponse } from 'next/server';
import { guardPlatformAdmin } from '@/lib/server/auth';
import { TTP_BASE_URL } from '@/lib/constants';
import { BundleDetailSchema } from '@/lib/api/types';
import type { BundleDetail } from '@/lib/api/types';

// Seed detail data returned when TTP is unavailable
const SEED_DETAILS: Record<string, BundleDetail> = {
  'bundle-essential': {
    id: 'bundle-essential',
    name: 'Essential Features',
    status: 'active',
    description: 'Core platform features included in every plan.',
    entitlements: [
      { id: 'ent-1', key: 'max_users', label: 'Max Users', value_type: 'number', value: 50 },
      { id: 'ent-2', key: 'api_access', label: 'API Access', value_type: 'boolean', value: true },
      { id: 'ent-3', key: 'storage_gb', label: 'Storage (GB)', value_type: 'number', value: 10 },
      { id: 'ent-4', key: 'support_tier', label: 'Support Tier', value_type: 'string', value: 'standard' },
      { id: 'ent-5', key: 'audit_log', label: 'Audit Log', value_type: 'boolean', value: true },
    ],
    created_at: '2026-01-01T00:00:00Z',
    updated_at: '2026-01-15T00:00:00Z',
  },
  'bundle-analytics': {
    id: 'bundle-analytics',
    name: 'Analytics Add-on',
    status: 'active',
    description: 'Advanced analytics and reporting capabilities.',
    entitlements: [
      { id: 'ent-6', key: 'custom_reports', label: 'Custom Reports', value_type: 'boolean', value: true },
      { id: 'ent-7', key: 'dashboard_limit', label: 'Dashboard Limit', value_type: 'number', value: 25 },
      { id: 'ent-8', key: 'data_export', label: 'Data Export', value_type: 'boolean', value: true },
    ],
    created_at: '2026-01-01T00:00:00Z',
    updated_at: '2026-01-10T00:00:00Z',
  },
  'bundle-compliance': {
    id: 'bundle-compliance',
    name: 'Compliance Suite',
    status: 'active',
    description: 'Regulatory compliance tools and audit features.',
    entitlements: [
      { id: 'ent-9', key: 'soc2_controls', label: 'SOC 2 Controls', value_type: 'boolean', value: true },
      { id: 'ent-10', key: 'gdpr_tools', label: 'GDPR Tools', value_type: 'boolean', value: true },
      { id: 'ent-11', key: 'retention_days', label: 'Retention (Days)', value_type: 'number', value: 365 },
      { id: 'ent-12', key: 'audit_frequency', label: 'Audit Frequency', value_type: 'string', value: 'monthly' },
      { id: 'ent-13', key: 'encryption_at_rest', label: 'Encryption at Rest', value_type: 'boolean', value: true },
      { id: 'ent-14', key: 'ip_allowlist', label: 'IP Allowlist', value_type: 'boolean', value: true },
      { id: 'ent-15', key: 'mfa_required', label: 'MFA Required', value_type: 'boolean', value: true },
      { id: 'ent-16', key: 'compliance_reports', label: 'Compliance Reports', value_type: 'boolean', value: true },
    ],
    created_at: '2026-01-15T00:00:00Z',
    updated_at: '2026-02-01T00:00:00Z',
  },
  'bundle-beta': {
    id: 'bundle-beta',
    name: 'Beta Features',
    status: 'draft',
    description: 'Experimental features available for early adopters.',
    entitlements: [
      { id: 'ent-17', key: 'ai_assist', label: 'AI Assist', value_type: 'boolean', value: true },
      { id: 'ent-18', key: 'realtime_collab', label: 'Real-time Collaboration', value_type: 'boolean', value: true },
    ],
    created_at: '2026-02-01T00:00:00Z',
    updated_at: '2026-02-10T00:00:00Z',
  },
};

export async function GET(
  _request: NextRequest,
  { params }: { params: Promise<{ bundle_id: string }> },
) {
  const auth = await guardPlatformAdmin();
  if (auth instanceof Response) return auth;

  const { bundle_id } = await params;

  try {
    const upstreamUrl = `${TTP_BASE_URL}/api/ttp/bundles/${encodeURIComponent(bundle_id)}`;
    const res = await fetch(upstreamUrl, {
      headers: { 'Content-Type': 'application/json' },
      signal: AbortSignal.timeout(5000),
    });

    if (!res.ok) {
      if (res.status === 404) {
        return NextResponse.json({ error: 'Bundle not found' }, { status: 404 });
      }
      return NextResponse.json(
        { error: `Upstream error: ${res.status}` },
        { status: res.status >= 500 ? 502 : res.status },
      );
    }

    const raw = await res.json();
    const parsed = BundleDetailSchema.safeParse(raw);
    if (parsed.success) {
      return NextResponse.json(parsed.data);
    }

    return NextResponse.json(raw);
  } catch {
    // TTP unavailable — return seed data if available
    const seed = SEED_DETAILS[bundle_id];
    if (seed) {
      return NextResponse.json(seed);
    }

    return NextResponse.json({ error: 'Bundle not found' }, { status: 404 });
  }
}
