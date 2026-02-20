// ============================================================
// RbacPanel — Roles & permissions assignment (tenant-scoped)
// Shows available roles and per-user role assignments.
// Grant/revoke require confirmation modal and always refetch.
// No optimistic updates — backend is source of truth.
// ============================================================
'use client';

import { useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { Button, Modal } from '@/components/ui';
import { REFETCH_INTERVAL_MS } from '@/lib/constants';
import type {
  RbacSnapshotResponse,
  RbacRole,
  RbacUserGrant,
} from '@/lib/api/types';

// ── Data fetchers ──────────────────────────────────────────

async function fetchRbacSnapshot(tenantId: string): Promise<RbacSnapshotResponse> {
  const res = await fetch(`/api/tenants/${encodeURIComponent(tenantId)}/rbac`);
  if (!res.ok) throw new Error(`HTTP ${res.status}`);
  return res.json();
}

async function grantRole(
  tenantId: string,
  userId: string,
  roleId: string,
): Promise<void> {
  const res = await fetch(
    `/api/tenants/${encodeURIComponent(tenantId)}/rbac/grant`,
    {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ user_id: userId, role_id: roleId, action: 'grant' }),
    },
  );
  if (!res.ok) {
    const body = await res.json().catch(() => ({ error: 'Unknown error' }));
    throw new Error(body.error ?? `HTTP ${res.status}`);
  }
}

async function revokeRole(
  tenantId: string,
  userId: string,
  roleId: string,
): Promise<void> {
  const res = await fetch(
    `/api/tenants/${encodeURIComponent(tenantId)}/rbac/revoke`,
    {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ user_id: userId, role_id: roleId, action: 'revoke' }),
    },
  );
  if (!res.ok) {
    const body = await res.json().catch(() => ({ error: 'Unknown error' }));
    throw new Error(body.error ?? `HTTP ${res.status}`);
  }
}

// ── Types ──────────────────────────────────────────────────

interface RbacPanelProps {
  tenantId: string;
}

interface RbacAction {
  type: 'grant' | 'revoke';
  user: RbacUserGrant;
  role: RbacRole;
}

// ── Component ──────────────────────────────────────────────

export function RbacPanel({ tenantId }: RbacPanelProps) {
  const queryClient = useQueryClient();
  const [pendingAction, setPendingAction] = useState<RbacAction | null>(null);

  const rbacQuery = useQuery({
    queryKey: ['tenant', tenantId, 'rbac'],
    queryFn: () => fetchRbacSnapshot(tenantId),
    refetchInterval: REFETCH_INTERVAL_MS,
  });

  const grantMutation = useMutation({
    mutationFn: ({ userId, roleId }: { userId: string; roleId: string }) =>
      grantRole(tenantId, userId, roleId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['tenant', tenantId, 'rbac'] });
      setPendingAction(null);
    },
  });

  const revokeMutation = useMutation({
    mutationFn: ({ userId, roleId }: { userId: string; roleId: string }) =>
      revokeRole(tenantId, userId, roleId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['tenant', tenantId, 'rbac'] });
      setPendingAction(null);
    },
  });

  const activeMutation = pendingAction?.type === 'grant' ? grantMutation : revokeMutation;

  const roles = rbacQuery.data?.roles ?? [];
  const userRoles = rbacQuery.data?.user_roles ?? [];

  function handleConfirm() {
    if (!pendingAction) return;
    const payload = { userId: pendingAction.user.user_id, roleId: pendingAction.role.id };
    if (pendingAction.type === 'grant') {
      grantMutation.mutate(payload);
    } else {
      revokeMutation.mutate(payload);
    }
  }

  return (
    <div data-testid="rbac-panel">
      <h2 className="text-lg font-semibold text-[--color-text-primary] mb-4">
        Roles & Permissions
      </h2>

      {rbacQuery.isLoading ? (
        <div
          className="rounded-[--radius-lg] border border-[--color-border-light] bg-[--color-bg-primary] p-8 text-center text-[--color-text-muted]"
          data-testid="rbac-loading"
        >
          Loading roles...
        </div>
      ) : rbacQuery.isError ? (
        <div
          className="rounded-[--radius-lg] border border-[--color-border-light] bg-[--color-bg-primary] p-8 text-center text-[--color-danger]"
          data-testid="rbac-error"
        >
          Unable to load roles
        </div>
      ) : (
        <div className="space-y-6">
          {/* Available Roles */}
          <RolesList roles={roles} />

          {/* Per-user Role Assignments */}
          <UserRolesTable
            userRoles={userRoles}
            roles={roles}
            onGrant={(user, role) => setPendingAction({ type: 'grant', user, role })}
            onRevoke={(user, role) => setPendingAction({ type: 'revoke', user, role })}
          />
        </div>
      )}

      {/* Confirmation modal */}
      <Modal
        isOpen={pendingAction !== null}
        title={pendingAction?.type === 'grant' ? 'Grant Role' : 'Revoke Role'}
        onClose={() => {
          setPendingAction(null);
          grantMutation.reset();
          revokeMutation.reset();
        }}
        size="sm"
      >
        <Modal.Body>
          <p className="text-sm text-[--color-text-primary]">
            {pendingAction?.type === 'grant' ? (
              <>
                Grant <strong>{pendingAction.role.name}</strong> to{' '}
                <strong>{pendingAction.user.email}</strong>?
              </>
            ) : (
              <>
                Revoke <strong>{pendingAction?.role.name}</strong> from{' '}
                <strong>{pendingAction?.user.email}</strong>?
                They will lose all permissions associated with this role.
              </>
            )}
          </p>
          {activeMutation.isError && (
            <p className="mt-3 text-sm text-[--color-danger]" data-testid="rbac-mutation-error">
              {activeMutation.error.message}
            </p>
          )}
        </Modal.Body>
        <Modal.Actions>
          <Button
            variant="ghost"
            size="sm"
            onClick={() => setPendingAction(null)}
            disableCooldown
          >
            Cancel
          </Button>
          <Button
            variant={pendingAction?.type === 'grant' ? 'primary' : 'danger'}
            size="sm"
            loading={activeMutation.isPending}
            onClick={handleConfirm}
            data-testid="rbac-confirm-btn"
          >
            {pendingAction?.type === 'grant' ? 'Grant' : 'Revoke'}
          </Button>
        </Modal.Actions>
      </Modal>
    </div>
  );
}

// ── Roles List ─────────────────────────────────────────────

function RolesList({ roles }: { roles: RbacRole[] }) {
  if (roles.length === 0) {
    return (
      <div
        className="rounded-[--radius-lg] border border-[--color-border-light] bg-[--color-bg-primary] p-6 text-center text-[--color-text-muted]"
        data-testid="rbac-roles-empty"
      >
        No roles defined for this tenant.
      </div>
    );
  }

  return (
    <div data-testid="rbac-roles-list">
      <h3 className="text-sm font-semibold text-[--color-text-secondary] mb-2">
        Available Roles
      </h3>
      <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
        {roles.map((role) => (
          <div
            key={role.id}
            className="rounded-[--radius-default] border border-[--color-border-light] bg-[--color-bg-primary] p-3"
            data-testid="rbac-role-card"
          >
            <div className="flex items-center justify-between mb-1">
              <span className="text-sm font-medium text-[--color-text-primary]">
                {role.name}
              </span>
              <span className="text-xs text-[--color-text-muted]">
                {role.permissions.length} permission{role.permissions.length !== 1 ? 's' : ''}
              </span>
            </div>
            {role.description && (
              <p className="text-xs text-[--color-text-secondary]">{role.description}</p>
            )}
            <div className="flex flex-wrap gap-1 mt-2">
              {role.permissions.map((perm) => (
                <span
                  key={perm}
                  className="px-2 py-0.5 text-xs rounded-full bg-[--color-bg-secondary] text-[--color-text-secondary]"
                  data-testid="rbac-permission-badge"
                >
                  {perm}
                </span>
              ))}
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}

// ── User Roles Table ───────────────────────────────────────

function UserRolesTable({
  userRoles,
  roles,
  onGrant,
  onRevoke,
}: {
  userRoles: RbacUserGrant[];
  roles: RbacRole[];
  onGrant: (user: RbacUserGrant, role: RbacRole) => void;
  onRevoke: (user: RbacUserGrant, role: RbacRole) => void;
}) {
  if (userRoles.length === 0) {
    return (
      <div
        className="rounded-[--radius-lg] border border-[--color-border-light] bg-[--color-bg-primary] p-6 text-center text-[--color-text-muted]"
        data-testid="rbac-users-empty"
      >
        No user role assignments found.
      </div>
    );
  }

  return (
    <div data-testid="rbac-user-roles">
      <h3 className="text-sm font-semibold text-[--color-text-secondary] mb-2">
        User Role Assignments
      </h3>
      <div className="rounded-[--radius-lg] border border-[--color-border-light] overflow-hidden">
        <table className="w-full border-collapse text-sm" data-testid="rbac-user-roles-table">
          <thead>
            <tr className="border-b border-[--color-border-light] bg-[--color-bg-secondary]">
              <th className="px-4 py-3 text-left text-xs font-semibold text-[--color-text-secondary] uppercase tracking-wide">
                User
              </th>
              <th className="px-4 py-3 text-left text-xs font-semibold text-[--color-text-secondary] uppercase tracking-wide">
                Current Roles
              </th>
              <th className="px-4 py-3 text-right text-xs font-semibold text-[--color-text-secondary] uppercase tracking-wide">
                Actions
              </th>
            </tr>
          </thead>
          <tbody>
            {userRoles.map((userGrant) => (
              <UserRoleRow
                key={userGrant.user_id}
                userGrant={userGrant}
                roles={roles}
                onGrant={onGrant}
                onRevoke={onRevoke}
              />
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}

// ── Per-user role row with inline grant/revoke ─────────────

function UserRoleRow({
  userGrant,
  roles,
  onGrant,
  onRevoke,
}: {
  userGrant: RbacUserGrant;
  roles: RbacRole[];
  onGrant: (user: RbacUserGrant, role: RbacRole) => void;
  onRevoke: (user: RbacUserGrant, role: RbacRole) => void;
}) {
  const [showGrantPicker, setShowGrantPicker] = useState(false);
  const assignedRoleIds = new Set(userGrant.roles);
  const grantableRoles = roles.filter((r) => !assignedRoleIds.has(r.id));
  const assignedRoles = roles.filter((r) => assignedRoleIds.has(r.id));

  return (
    <tr
      className="border-b border-[--color-border-light] hover:bg-[--color-bg-secondary] transition-[--transition-fast]"
      data-testid="rbac-user-row"
    >
      <td className="px-4 py-3">
        <div className="text-[--color-text-primary]">{userGrant.email}</div>
        {userGrant.name && (
          <div className="text-xs text-[--color-text-secondary]">{userGrant.name}</div>
        )}
      </td>
      <td className="px-4 py-3">
        <div className="flex flex-wrap gap-1.5">
          {assignedRoles.length === 0 ? (
            <span className="text-xs text-[--color-text-muted]">No roles</span>
          ) : (
            assignedRoles.map((role) => (
              <span
                key={role.id}
                className="inline-flex items-center gap-1 px-2.5 py-0.5 text-xs font-medium rounded-full bg-blue-100 text-blue-800"
                data-testid="rbac-assigned-role"
              >
                {role.name}
                <Button
                  variant="ghost"
                  size="xs"
                  onClick={() => onRevoke(userGrant, role)}
                  className="ml-0.5 !p-0 !min-w-0 text-blue-600 hover:text-red-600 font-bold"
                  title={`Revoke ${role.name}`}
                  data-testid="rbac-revoke-btn"
                  disableCooldown
                >
                  ×
                </Button>
              </span>
            ))
          )}
        </div>
      </td>
      <td className="px-4 py-3 text-right">
        {grantableRoles.length > 0 && (
          <div className="relative inline-block">
            <Button
              variant="outline"
              size="xs"
              onClick={() => setShowGrantPicker(!showGrantPicker)}
              data-testid="rbac-grant-btn"
            >
              Grant Role
            </Button>
            {showGrantPicker && (
              <div
                className="absolute right-0 top-full mt-1 z-10 min-w-[180px] rounded-[--radius-default] border border-[--color-border-light] bg-[--color-bg-primary] shadow-lg"
                data-testid="rbac-grant-picker"
              >
                {grantableRoles.map((role) => (
                  <Button
                    key={role.id}
                    variant="ghost"
                    size="sm"
                    onClick={() => {
                      setShowGrantPicker(false);
                      onGrant(userGrant, role);
                    }}
                    className="!block w-full !text-left !rounded-none !px-3 !py-2 text-sm text-[--color-text-primary] hover:bg-[--color-bg-secondary]"
                    data-testid="rbac-grant-option"
                    disableCooldown
                  >
                    {role.name}
                  </Button>
                ))}
              </div>
            )}
          </div>
        )}
      </td>
    </tr>
  );
}
