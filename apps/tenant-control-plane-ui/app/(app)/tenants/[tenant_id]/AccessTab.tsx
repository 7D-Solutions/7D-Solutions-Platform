// ============================================================
// AccessTab — Users list + RBAC + Support Sessions
// Shows tenant-scoped users (with deactivation), RBAC panel,
// and support session start/stop with reason.
// ============================================================
'use client';

import { useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { Button, Modal, StatusBadge, FormTextarea } from '@/components/ui';
import { formatDate } from '@/infrastructure/utils/formatters';
import { REFETCH_INTERVAL_MS } from '@/lib/constants';
import type { TenantUser, TenantUserListResponse } from '@/lib/api/types';
import { RbacPanel } from './RbacPanel';

// ── Support session helpers ──────────────────────────────────

interface MeResponse {
  sub: string;
  email: string;
  roles: string[];
  actor_type: 'staff' | 'support';
  support_tenant_id?: string;
}

async function fetchMe(): Promise<MeResponse> {
  const res = await fetch('/api/auth/me');
  if (!res.ok) throw new Error(`HTTP ${res.status}`);
  return res.json();
}

async function startSupportSession(tenantId: string, reason: string): Promise<void> {
  const res = await fetch(
    `/api/tenants/${encodeURIComponent(tenantId)}/support-sessions/start`,
    {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ reason }),
    },
  );
  if (!res.ok) {
    const body = await res.json().catch(() => ({ error: 'Unknown error' }));
    throw new Error(body.error ?? `HTTP ${res.status}`);
  }
}

async function endSupportSession(tenantId: string): Promise<void> {
  const res = await fetch(
    `/api/tenants/${encodeURIComponent(tenantId)}/support-sessions/end`,
    { method: 'POST' },
  );
  if (!res.ok) {
    const body = await res.json().catch(() => ({ error: 'Unknown error' }));
    throw new Error(body.error ?? `HTTP ${res.status}`);
  }
}

// ── Data fetchers ──────────────────────────────────────────

async function fetchUsers(tenantId: string): Promise<TenantUserListResponse> {
  const res = await fetch(`/api/tenants/${encodeURIComponent(tenantId)}/users`);
  if (!res.ok) throw new Error(`HTTP ${res.status}`);
  return res.json();
}

async function deactivateUser(tenantId: string, userId: string): Promise<void> {
  const res = await fetch(
    `/api/tenants/${encodeURIComponent(tenantId)}/users/${encodeURIComponent(userId)}/deactivate`,
    { method: 'POST' },
  );
  if (!res.ok) {
    const body = await res.json().catch(() => ({ error: 'Unknown error' }));
    throw new Error(body.error ?? `HTTP ${res.status}`);
  }
}

// ── Component ──────────────────────────────────────────────

interface AccessTabProps {
  tenantId: string;
}

export function AccessTab({ tenantId }: AccessTabProps) {
  const queryClient = useQueryClient();
  const [confirmUser, setConfirmUser] = useState<TenantUser | null>(null);

  // Support session state
  const [showStartModal, setShowStartModal] = useState(false);
  const [supportReason, setSupportReason] = useState('');

  const meQuery = useQuery({
    queryKey: ['auth', 'me'],
    queryFn: fetchMe,
    refetchInterval: REFETCH_INTERVAL_MS,
  });

  const isSupportActive =
    meQuery.data?.actor_type === 'support' &&
    meQuery.data?.support_tenant_id === tenantId;

  const startMutation = useMutation({
    mutationFn: () => startSupportSession(tenantId, supportReason),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['auth', 'me'] });
      setShowStartModal(false);
      setSupportReason('');
    },
  });

  const endMutation = useMutation({
    mutationFn: () => endSupportSession(tenantId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['auth', 'me'] });
    },
  });

  const usersQuery = useQuery({
    queryKey: ['tenant', tenantId, 'users'],
    queryFn: () => fetchUsers(tenantId),
    refetchInterval: REFETCH_INTERVAL_MS,
  });

  const deactivateMutation = useMutation({
    mutationFn: (userId: string) => deactivateUser(tenantId, userId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['tenant', tenantId, 'users'] });
      setConfirmUser(null);
    },
  });

  const users = usersQuery.data?.users ?? [];

  return (
    <div data-testid="access-tab" className="space-y-8">
      <h2 className="text-lg font-semibold text-[--color-text-primary] mb-4">Users</h2>

      {usersQuery.isLoading ? (
        <div className="rounded-[--radius-lg] border border-[--color-border-light] bg-[--color-bg-primary] p-8 text-center text-[--color-text-muted]">
          Loading users...
        </div>
      ) : usersQuery.isError ? (
        <div className="rounded-[--radius-lg] border border-[--color-border-light] bg-[--color-bg-primary] p-8 text-center text-[--color-danger]" data-testid="users-error">
          Unable to load users
        </div>
      ) : users.length === 0 ? (
        <div className="rounded-[--radius-lg] border border-[--color-border-light] bg-[--color-bg-primary] p-8 text-center text-[--color-text-muted]" data-testid="users-empty">
          No users found for this tenant.
        </div>
      ) : (
        <div className="rounded-[--radius-lg] border border-[--color-border-light] overflow-hidden" data-testid="users-table">
          <table className="w-full border-collapse text-sm">
            <thead>
              <tr className="border-b border-[--color-border-light] bg-[--color-bg-secondary]">
                <th className="px-4 py-3 text-left text-xs font-semibold text-[--color-text-secondary] uppercase tracking-wide">Email</th>
                <th className="px-4 py-3 text-left text-xs font-semibold text-[--color-text-secondary] uppercase tracking-wide">Name</th>
                <th className="px-4 py-3 text-left text-xs font-semibold text-[--color-text-secondary] uppercase tracking-wide">Status</th>
                <th className="px-4 py-3 text-left text-xs font-semibold text-[--color-text-secondary] uppercase tracking-wide">Last Seen</th>
                <th className="px-4 py-3 text-right text-xs font-semibold text-[--color-text-secondary] uppercase tracking-wide">Actions</th>
              </tr>
            </thead>
            <tbody>
              {users.map((user) => (
                <tr
                  key={user.id}
                  className="border-b border-[--color-border-light] hover:bg-[--color-bg-secondary] transition-[--transition-fast]"
                  data-testid="user-row"
                >
                  <td className="px-4 py-3 text-[--color-text-primary]">{user.email}</td>
                  <td className="px-4 py-3 text-[--color-text-primary]">{user.name ?? '—'}</td>
                  <td className="px-4 py-3">
                    <StatusBadge status={user.status} />
                  </td>
                  <td className="px-4 py-3 text-[--color-text-secondary]">
                    {user.last_seen ? formatDate(user.last_seen) : '—'}
                  </td>
                  <td className="px-4 py-3 text-right">
                    {user.status === 'active' && (
                      <Button
                        variant="danger"
                        size="xs"
                        onClick={() => setConfirmUser(user)}
                        data-testid="deactivate-btn"
                      >
                        Deactivate
                      </Button>
                    )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      {/* Deactivation confirmation modal */}
      <Modal
        isOpen={confirmUser !== null}
        title="Deactivate User"
        onClose={() => setConfirmUser(null)}
        size="sm"
      >
        <Modal.Body>
          <p className="text-sm text-[--color-text-primary]">
            Are you sure you want to deactivate{' '}
            <strong>{confirmUser?.email}</strong>?
            They will no longer be able to sign in.
          </p>
          {deactivateMutation.isError && (
            <p className="mt-3 text-sm text-[--color-danger]" data-testid="deactivate-error">
              {deactivateMutation.error.message}
            </p>
          )}
        </Modal.Body>
        <Modal.Actions>
          <Button
            variant="ghost"
            size="sm"
            onClick={() => setConfirmUser(null)}
            // Cancel is not a mutation — no cooldown needed
            disableCooldown
          >
            Cancel
          </Button>
          <Button
            variant="danger"
            size="sm"
            loading={deactivateMutation.isPending}
            onClick={() => {
              if (confirmUser) deactivateMutation.mutate(confirmUser.id);
            }}
            data-testid="confirm-deactivate-btn"
          >
            Deactivate
          </Button>
        </Modal.Actions>
      </Modal>

      {/* Support Sessions */}
      <section
        className="rounded-[--radius-lg] border border-[--color-border-light] bg-[--color-bg-primary] p-5"
        data-testid="support-session-section"
      >
        <h2 className="text-sm font-semibold text-[--color-text-primary] mb-3 pb-2 border-b border-[--color-border-light]">
          Support Sessions
        </h2>

        {isSupportActive ? (
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-2">
              <StatusBadge status="active" />
              <span className="text-sm text-[--color-text-primary]">
                Support session active for this tenant
              </span>
            </div>
            <Button
              variant="warning"
              size="sm"
              loading={endMutation.isPending}
              onClick={() => endMutation.mutate()}
              data-testid="end-support-session-btn"
            >
              End Session
            </Button>
          </div>
        ) : (
          <div className="flex items-center justify-between">
            <p className="text-sm text-[--color-text-muted]">
              No active support session. Start one to access this tenant as a support agent.
            </p>
            <Button
              variant="primary"
              size="sm"
              onClick={() => setShowStartModal(true)}
              data-testid="start-support-session-btn"
            >
              Start Support Session
            </Button>
          </div>
        )}

        {endMutation.isError && (
          <p className="mt-3 text-sm text-[--color-danger]" data-testid="support-session-error">
            {endMutation.error.message}
          </p>
        )}
      </section>

      {/* Start Support Session Modal */}
      <Modal
        isOpen={showStartModal}
        title="Start Support Session"
        onClose={() => {
          setShowStartModal(false);
          setSupportReason('');
          startMutation.reset();
        }}
        size="md"
      >
        <Modal.Body>
          <p className="text-sm text-[--color-text-primary] mb-4">
            Starting a support session grants you temporary access to act on
            behalf of this tenant. A reason is required for audit purposes.
          </p>
          <FormTextarea
            label="Reason for support session"
            required
            placeholder="Describe why you need to access this tenant..."
            value={supportReason}
            onChange={(e) => setSupportReason(e.target.value)}
            maxLength={500}
            showCharCount
            data-testid="support-reason-input"
          />
          {startMutation.isError && (
            <p className="mt-3 text-sm text-[--color-danger]" data-testid="start-session-error">
              {startMutation.error.message}
            </p>
          )}
        </Modal.Body>
        <Modal.Actions>
          <Button
            variant="ghost"
            size="sm"
            onClick={() => {
              setShowStartModal(false);
              setSupportReason('');
              startMutation.reset();
            }}
            disableCooldown
          >
            Cancel
          </Button>
          <Button
            variant="primary"
            size="sm"
            disabled={!supportReason.trim()}
            loading={startMutation.isPending}
            onClick={() => startMutation.mutate()}
            data-testid="confirm-start-session-btn"
          >
            Start Session
          </Button>
        </Modal.Actions>
      </Modal>

      {/* RBAC Roles & Permissions */}
      <RbacPanel tenantId={tenantId} />
    </div>
  );
}
