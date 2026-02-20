// ============================================================
// GET /api/tenants/[tenant_id]/rbac — RBAC snapshot
// Returns aggregated roles and per-user role assignments.
// Proxies to identity-auth; falls back to seed data.
// Auth: requires platform_admin JWT in httpOnly cookie
// ============================================================
import { NextRequest, NextResponse } from 'next/server';
import { guardPlatformAdmin } from '@/lib/server/auth';
import { IDENTITY_AUTH_BASE_URL } from '@/lib/constants';
import { RbacSnapshotResponseSchema } from '@/lib/api/types';
import type { RbacSnapshotResponse } from '@/lib/api/types';

export async function GET(
  _request: NextRequest,
  { params }: { params: Promise<{ tenant_id: string }> }
) {
  const auth = await guardPlatformAdmin();
  if (auth instanceof Response) return auth;

  const { tenant_id } = await params;

  try {
    const upstreamUrl =
      `${IDENTITY_AUTH_BASE_URL}/api/tenants/${encodeURIComponent(tenant_id)}/rbac`;
    const res = await fetch(upstreamUrl, {
      headers: { 'Content-Type': 'application/json' },
      signal: AbortSignal.timeout(5000),
    });

    if (res.ok) {
      const raw = await res.json();
      const parsed = RbacSnapshotResponseSchema.safeParse(raw);
      if (parsed.success) {
        return NextResponse.json(parsed.data);
      }
      return NextResponse.json(raw);
    }
    // Upstream not available — fall through to seed data
  } catch {
    // identity-auth unavailable — fall through to seed data
  }

  // Seed data so the UI renders without a live RBAC endpoint
  const fallback: RbacSnapshotResponse = {
    roles: [
      {
        id: 'role-admin',
        name: 'Admin',
        description: 'Full access to all tenant resources',
        permissions: ['read', 'write', 'delete', 'manage_users'],
      },
      {
        id: 'role-editor',
        name: 'Editor',
        description: 'Can create and edit resources',
        permissions: ['read', 'write'],
      },
      {
        id: 'role-viewer',
        name: 'Viewer',
        description: 'Read-only access to tenant resources',
        permissions: ['read'],
      },
      {
        id: 'role-billing',
        name: 'Billing Manager',
        description: 'Manage billing and invoices',
        permissions: ['read', 'billing_manage'],
      },
    ],
    user_roles: [
      {
        user_id: 'seed-user-001',
        email: 'admin@example.com',
        name: 'Admin User',
        roles: ['role-admin'],
      },
      {
        user_id: 'seed-user-002',
        email: 'viewer@example.com',
        name: 'Viewer User',
        roles: ['role-viewer'],
      },
      {
        user_id: 'seed-user-003',
        email: 'suspended@example.com',
        name: 'Suspended User',
        roles: [],
      },
    ],
  };
  return NextResponse.json(fallback);
}
