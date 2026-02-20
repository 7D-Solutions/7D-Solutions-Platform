// ============================================================
// SettingsTab — Lifecycle actions, plan change entrypoint, config placeholders
// Suspend/activate require simple confirmation.
// Terminate requires reason + password re-auth before final confirm.
// ============================================================
'use client';

import { useState } from 'react';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { Button, Modal, StatusBadge, FormInput, FormTextarea } from '@/components/ui';
import type { TenantDetail } from '@/lib/api/types';

// ── API helpers ─────────────────────────────────────────────

async function suspendTenant(tenantId: string): Promise<void> {
  const res = await fetch(
    `/api/tenants/${encodeURIComponent(tenantId)}/suspend`,
    { method: 'POST' },
  );
  if (!res.ok) {
    const body = await res.json().catch(() => ({ error: 'Unknown error' }));
    throw new Error(body.error ?? `HTTP ${res.status}`);
  }
}

async function activateTenant(tenantId: string): Promise<void> {
  const res = await fetch(
    `/api/tenants/${encodeURIComponent(tenantId)}/activate`,
    { method: 'POST' },
  );
  if (!res.ok) {
    const body = await res.json().catch(() => ({ error: 'Unknown error' }));
    throw new Error(body.error ?? `HTTP ${res.status}`);
  }
}

async function terminateTenant(tenantId: string, reason: string): Promise<void> {
  const res = await fetch(
    `/api/tenants/${encodeURIComponent(tenantId)}/terminate`,
    { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ reason }) },
  );
  if (!res.ok) {
    const body = await res.json().catch(() => ({ error: 'Unknown error' }));
    throw new Error(body.error ?? `HTTP ${res.status}`);
  }
}

async function reauthPassword(password: string): Promise<void> {
  const res = await fetch('/api/auth/reauth', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ password }),
  });
  if (!res.ok) {
    const body = await res.json().catch(() => ({ error: 'Unknown error' }));
    throw new Error(body.error ?? `HTTP ${res.status}`);
  }
}

// ── Types ───────────────────────────────────────────────────

type LifecycleAction = 'suspend' | 'activate' | null;

interface SettingsTabProps {
  tenantId: string;
  tenant: TenantDetail | undefined;
}

// ── Component ───────────────────────────────────────────────

export function SettingsTab({ tenantId, tenant }: SettingsTabProps) {
  const queryClient = useQueryClient();

  // Suspend/activate modal
  const [lifecycleAction, setLifecycleAction] = useState<LifecycleAction>(null);

  // Terminate modal state machine: idle → reason → reauth → confirm
  const [terminateStep, setTerminateStep] = useState<'idle' | 'reason' | 'reauth' | 'confirm'>('idle');
  const [terminateReason, setTerminateReason] = useState('');
  const [reauthPassword_, setReauthPassword] = useState('');
  const [reauthError, setReauthError] = useState('');
  const [reauthDone, setReauthDone] = useState(false);

  const status = tenant?.status ?? 'unknown';
  const canSuspend = ['active', 'trial', 'unknown'].includes(status);
  const canActivate = status === 'suspended';
  const canTerminate = status !== 'terminated';

  // ── Mutations ───────────────────────────────────────────

  const suspendMutation = useMutation({
    mutationFn: () => suspendTenant(tenantId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['tenant', tenantId] });
      queryClient.invalidateQueries({ queryKey: ['tenant-list'] });
      setLifecycleAction(null);
    },
  });

  const activateMutation = useMutation({
    mutationFn: () => activateTenant(tenantId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['tenant', tenantId] });
      queryClient.invalidateQueries({ queryKey: ['tenant-list'] });
      setLifecycleAction(null);
    },
  });

  const reauthMutation = useMutation({
    mutationFn: () => reauthPassword(reauthPassword_),
    onSuccess: () => {
      setReauthDone(true);
      setReauthError('');
      setTerminateStep('confirm');
    },
    onError: (err: Error) => {
      setReauthError(err.message || 'Re-authentication failed');
    },
  });

  const terminateMutation = useMutation({
    mutationFn: () => terminateTenant(tenantId, terminateReason),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['tenant', tenantId] });
      queryClient.invalidateQueries({ queryKey: ['tenant-list'] });
      resetTerminate();
    },
  });

  function resetTerminate() {
    setTerminateStep('idle');
    setTerminateReason('');
    setReauthPassword('');
    setReauthError('');
    setReauthDone(false);
  }

  const activeMutation = lifecycleAction === 'suspend' ? suspendMutation : activateMutation;

  // ── Render ──────────────────────────────────────────────

  return (
    <div data-testid="settings-tab" className="space-y-6">
      {/* Lifecycle Actions */}
      <section
        className="rounded-[--radius-lg] border border-[--color-border-light] bg-[--color-bg-primary] p-5"
        data-testid="lifecycle-section"
      >
        <h2 className="text-sm font-semibold text-[--color-text-primary] mb-3 pb-2 border-b border-[--color-border-light]">
          Lifecycle Actions
        </h2>
        <div className="flex items-center gap-2 mb-3">
          <span className="text-sm text-[--color-text-secondary]">Current status:</span>
          <StatusBadge status={status} />
        </div>
        <div className="flex flex-wrap gap-3">
          {canSuspend && (
            <Button
              variant="warning"
              size="sm"
              onClick={() => setLifecycleAction('suspend')}
              data-testid="suspend-btn"
            >
              Suspend Tenant
            </Button>
          )}
          {canActivate && (
            <Button
              variant="success"
              size="sm"
              onClick={() => setLifecycleAction('activate')}
              data-testid="activate-btn"
            >
              Activate Tenant
            </Button>
          )}
          {canTerminate && (
            <Button
              variant="danger"
              size="sm"
              onClick={() => setTerminateStep('reason')}
              data-testid="terminate-btn"
            >
              Terminate Tenant
            </Button>
          )}
          {!canSuspend && !canActivate && !canTerminate && (
            <p className="text-sm text-[--color-text-muted]">
              No lifecycle actions available for this tenant.
            </p>
          )}
        </div>
      </section>

      {/* Plan Change */}
      <section
        className="rounded-[--radius-lg] border border-[--color-border-light] bg-[--color-bg-primary] p-5"
        data-testid="plan-change-section"
      >
        <h2 className="text-sm font-semibold text-[--color-text-primary] mb-3 pb-2 border-b border-[--color-border-light]">
          Plan
        </h2>
        <div className="flex items-center justify-between">
          <div>
            <p className="text-sm text-[--color-text-primary]">
              Current plan: <strong>{tenant?.plan ?? 'Unknown'}</strong>
            </p>
          </div>
          <Button
            variant="outline"
            size="sm"
            disabled
            data-testid="change-plan-btn"
          >
            Change Plan
          </Button>
        </div>
        <p className="text-xs text-[--color-text-muted] mt-2">
          Plan changes will be available in a future update.
        </p>
      </section>

      {/* Account Configuration */}
      <section
        className="rounded-[--radius-lg] border border-[--color-border-light] bg-[--color-bg-primary] p-5"
        data-testid="account-config-section"
      >
        <h2 className="text-sm font-semibold text-[--color-text-primary] mb-3 pb-2 border-b border-[--color-border-light]">
          Account Configuration
        </h2>
        <p className="text-sm text-[--color-text-muted]">
          Account configuration options will be available in a future update.
        </p>
      </section>

      {/* ── Suspend/Activate Confirmation Modal ────────────── */}
      <Modal
        isOpen={lifecycleAction !== null}
        title={lifecycleAction === 'suspend' ? 'Suspend Tenant' : 'Activate Tenant'}
        onClose={() => {
          setLifecycleAction(null);
          suspendMutation.reset();
          activateMutation.reset();
        }}
        size="sm"
      >
        <Modal.Body>
          <p className="text-sm text-[--color-text-primary]">
            {lifecycleAction === 'suspend'
              ? `Are you sure you want to suspend "${tenant?.name ?? tenantId}"? Users will lose access until the tenant is reactivated.`
              : `Are you sure you want to activate "${tenant?.name ?? tenantId}"? Users will regain access immediately.`}
          </p>
          {activeMutation.isError && (
            <p className="mt-3 text-sm text-[--color-danger]" data-testid="lifecycle-error">
              {activeMutation.error.message}
            </p>
          )}
        </Modal.Body>
        <Modal.Actions>
          <Button
            variant="ghost"
            size="sm"
            onClick={() => setLifecycleAction(null)}
            // Cancel is not a mutation
            disableCooldown
          >
            Cancel
          </Button>
          <Button
            variant={lifecycleAction === 'suspend' ? 'warning' : 'success'}
            size="sm"
            loading={activeMutation.isPending}
            onClick={() => activeMutation.mutate()}
            data-testid="confirm-lifecycle-btn"
          >
            {lifecycleAction === 'suspend' ? 'Suspend' : 'Activate'}
          </Button>
        </Modal.Actions>
      </Modal>

      {/* ── Terminate Modal (multi-step) ───────────────────── */}
      <Modal
        isOpen={terminateStep !== 'idle'}
        title="Terminate Tenant"
        onClose={resetTerminate}
        size="md"
        preventClosing={terminateStep === 'confirm'}
      >
        {/* Step 1: Reason */}
        {terminateStep === 'reason' && (
          <>
            <Modal.Body>
              <p className="text-sm text-[--color-text-primary] mb-4">
                Terminating <strong>{tenant?.name ?? tenantId}</strong> is permanent.
                All data will be scheduled for deletion.
              </p>
              <FormTextarea
                label="Reason for termination"
                required
                placeholder="Explain why this tenant is being terminated..."
                value={terminateReason}
                onChange={(e) => setTerminateReason(e.target.value)}
                maxLength={500}
                showCharCount
                data-testid="terminate-reason-input"
              />
            </Modal.Body>
            <Modal.Actions>
              <Button
                variant="ghost"
                size="sm"
                onClick={resetTerminate}
                disableCooldown
              >
                Cancel
              </Button>
              <Button
                variant="danger"
                size="sm"
                disabled={!terminateReason.trim()}
                onClick={() => setTerminateStep('reauth')}
                data-testid="terminate-next-btn"
              >
                Next: Verify Identity
              </Button>
            </Modal.Actions>
          </>
        )}

        {/* Step 2: Re-auth */}
        {terminateStep === 'reauth' && (
          <>
            <Modal.Body>
              <p className="text-sm text-[--color-text-primary] mb-4">
                To confirm termination, enter your password.
              </p>
              <FormInput
                label="Password"
                type="password"
                required
                placeholder="Enter your password"
                value={reauthPassword_}
                onChange={(e) => {
                  setReauthPassword(e.target.value);
                  setReauthError('');
                }}
                error={reauthError}
                data-testid="reauth-password-input"
              />
            </Modal.Body>
            <Modal.Actions>
              <Button
                variant="ghost"
                size="sm"
                onClick={() => setTerminateStep('reason')}
                disableCooldown
              >
                Back
              </Button>
              <Button
                variant="danger"
                size="sm"
                loading={reauthMutation.isPending}
                disabled={!reauthPassword_.trim()}
                onClick={() => reauthMutation.mutate()}
                data-testid="reauth-verify-btn"
              >
                Verify
              </Button>
            </Modal.Actions>
          </>
        )}

        {/* Step 3: Final confirm */}
        {terminateStep === 'confirm' && reauthDone && (
          <>
            <Modal.Body>
              <div
                className="rounded-[--radius-default] border border-[--color-danger] bg-red-50 p-4 mb-4"
                data-testid="terminate-warning"
              >
                <p className="text-sm font-semibold text-[--color-danger] mb-1">
                  This action cannot be undone.
                </p>
                <p className="text-sm text-[--color-text-primary]">
                  Tenant <strong>{tenant?.name ?? tenantId}</strong> will be permanently terminated.
                </p>
              </div>
              <p className="text-sm text-[--color-text-secondary]">
                <strong>Reason:</strong> {terminateReason}
              </p>
              {terminateMutation.isError && (
                <p className="mt-3 text-sm text-[--color-danger]" data-testid="terminate-error">
                  {terminateMutation.error.message}
                </p>
              )}
            </Modal.Body>
            <Modal.Actions>
              <Button
                variant="ghost"
                size="sm"
                onClick={resetTerminate}
                disableCooldown
              >
                Cancel
              </Button>
              <Button
                variant="danger"
                size="sm"
                loading={terminateMutation.isPending}
                onClick={() => terminateMutation.mutate()}
                data-testid="confirm-terminate-btn"
              >
                Terminate Tenant
              </Button>
            </Modal.Actions>
          </>
        )}
      </Modal>
    </div>
  );
}
