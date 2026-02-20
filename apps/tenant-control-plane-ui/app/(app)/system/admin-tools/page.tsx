// ============================================================
// /app/system/admin-tools — Platform admin tools
// Two tools: Run billing now + Reconcile tenant mapping.
// Both require confirmation with reason capture.
// Gracefully handles backend unavailability (not-available state).
// ============================================================
'use client';

import { useState } from 'react';
import { useForm } from 'react-hook-form';
import { zodResolver } from '@hookform/resolvers/zod';
import { useMutation } from '@tanstack/react-query';
import { Zap, RefreshCw } from 'lucide-react';
import { Button, Modal, FormInput, FormTextarea } from '@/components/ui';
import {
  RunBillingRequestSchema,
  ReconcileMappingRequestSchema,
} from '@/lib/api/types';
import type {
  RunBillingRequest,
  ReconcileMappingRequest,
  AdminToolResult,
} from '@/lib/api/types';

// ── API helpers ─────────────────────────────────────────────

async function runBilling(payload: RunBillingRequest): Promise<AdminToolResult> {
  const res = await fetch('/api/system/run-billing', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(payload),
  });
  const data = await res.json();
  if (!res.ok && !data.not_available) {
    throw new Error(data.error ?? data.message ?? `HTTP ${res.status}`);
  }
  return data;
}

async function reconcileMapping(payload: ReconcileMappingRequest): Promise<AdminToolResult> {
  const res = await fetch('/api/system/reconcile-tenant-mapping', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(payload),
  });
  const data = await res.json();
  if (!res.ok && !data.not_available) {
    throw new Error(data.error ?? data.message ?? `HTTP ${res.status}`);
  }
  return data;
}

// ── Result Panel ────────────────────────────────────────────

function ResultPanel({ result, error, onDismiss }: {
  result: AdminToolResult | undefined;
  error: Error | null;
  onDismiss: () => void;
}) {
  if (!result && !error) return null;

  if (error) {
    return (
      <div
        className="rounded-[--radius-default] border border-[--color-danger] bg-red-50 p-4 mt-4"
        data-testid="tool-result-error"
      >
        <p className="text-sm font-medium text-[--color-danger]">Error</p>
        <p className="text-sm text-[--color-text-primary] mt-1">{error.message}</p>
        <Button variant="ghost" size="xs" onClick={onDismiss} className="mt-2" disableCooldown>
          Dismiss
        </Button>
      </div>
    );
  }

  if (result?.not_available) {
    return (
      <div
        className="rounded-[--radius-default] border border-yellow-300 bg-yellow-50 p-4 mt-4"
        data-testid="tool-result-not-available"
      >
        <p className="text-sm font-medium text-yellow-800">Not available in this environment</p>
        <p className="text-sm text-[--color-text-primary] mt-1">{result.message}</p>
        <Button variant="ghost" size="xs" onClick={onDismiss} className="mt-2" disableCooldown>
          Dismiss
        </Button>
      </div>
    );
  }

  if (result?.ok) {
    return (
      <div
        className="rounded-[--radius-default] border border-green-300 bg-green-50 p-4 mt-4"
        data-testid="tool-result-success"
      >
        <p className="text-sm font-medium text-green-800">Success</p>
        <p className="text-sm text-[--color-text-primary] mt-1">{result.message}</p>
        <Button variant="ghost" size="xs" onClick={onDismiss} className="mt-2" disableCooldown>
          Dismiss
        </Button>
      </div>
    );
  }

  return null;
}

// ── Run Billing Tool ────────────────────────────────────────

function RunBillingTool() {
  const [showConfirm, setShowConfirm] = useState(false);

  const form = useForm<RunBillingRequest>({
    resolver: zodResolver(RunBillingRequestSchema),
    defaultValues: { tenant_id: '', reason: '' },
  });

  const mutation = useMutation({
    mutationFn: runBilling,
    onSuccess: () => {
      setShowConfirm(false);
      form.reset();
    },
    onError: () => {
      setShowConfirm(false);
    },
  });

  const handleFormSubmit = form.handleSubmit(() => {
    setShowConfirm(true);
  });

  const handleConfirm = () => {
    mutation.mutate(form.getValues());
  };

  const values = form.watch();

  return (
    <div
      className="rounded-[--radius-lg] border border-[--color-border-light] bg-[--color-bg-primary] p-5"
      data-testid="run-billing-tool"
    >
      <div className="flex items-center gap-3 mb-4 pb-3 border-b border-[--color-border-light]">
        <Zap className="h-5 w-5 text-[--color-primary]" />
        <h2 className="text-sm font-semibold text-[--color-text-primary]">Run Billing Now</h2>
      </div>

      <p className="text-sm text-[--color-text-secondary] mb-4">
        Trigger an immediate billing cycle. Leave Tenant ID blank to run for all tenants.
      </p>

      <form onSubmit={handleFormSubmit} className="space-y-4">
        <FormInput
          label="Tenant ID"
          placeholder="Optional — leave blank for all tenants"
          {...form.register('tenant_id')}
          error={form.formState.errors.tenant_id?.message}
          data-testid="billing-tenant-id"
        />

        <FormTextarea
          label="Reason"
          required
          placeholder="Why is this billing run needed?"
          maxLength={500}
          showCharCount
          value={values.reason}
          {...form.register('reason')}
          error={form.formState.errors.reason?.message}
          data-testid="billing-reason"
        />

        <Button
          type="submit"
          variant="primary"
          size="sm"
          disabled={mutation.isPending}
          data-testid="billing-submit-btn"
        >
          Review & Run
        </Button>
      </form>

      <ResultPanel
        result={mutation.data}
        error={mutation.error}
        onDismiss={() => mutation.reset()}
      />

      {/* Confirmation modal */}
      <Modal
        isOpen={showConfirm}
        title="Confirm: Run Billing"
        onClose={() => setShowConfirm(false)}
        size="sm"
      >
        <Modal.Body>
          <div className="space-y-3" data-testid="billing-confirm-summary">
            <p className="text-sm text-[--color-text-primary]">
              You are about to trigger an immediate billing cycle.
            </p>
            <dl className="space-y-2 text-sm">
              <div>
                <dt className="font-medium text-[--color-text-secondary]">Tenant</dt>
                <dd className="text-[--color-text-primary]" data-testid="confirm-tenant-value">
                  {values.tenant_id || 'All tenants'}
                </dd>
              </div>
              <div>
                <dt className="font-medium text-[--color-text-secondary]">Reason</dt>
                <dd className="text-[--color-text-primary]" data-testid="confirm-reason-value">
                  {values.reason}
                </dd>
              </div>
            </dl>
          </div>
        </Modal.Body>
        <Modal.Actions>
          <Button
            variant="ghost"
            size="sm"
            onClick={() => setShowConfirm(false)}
            disableCooldown
          >
            Cancel
          </Button>
          <Button
            variant="primary"
            size="sm"
            loading={mutation.isPending}
            onClick={handleConfirm}
            data-testid="billing-confirm-btn"
          >
            Run Billing
          </Button>
        </Modal.Actions>
      </Modal>
    </div>
  );
}

// ── Reconcile Tenant Mapping Tool ───────────────────────────

function ReconcileMappingTool() {
  const [showConfirm, setShowConfirm] = useState(false);

  const form = useForm<ReconcileMappingRequest>({
    resolver: zodResolver(ReconcileMappingRequestSchema),
    defaultValues: { tenant_id: '', reason: '' },
  });

  const mutation = useMutation({
    mutationFn: reconcileMapping,
    onSuccess: () => {
      setShowConfirm(false);
      form.reset();
    },
    onError: () => {
      setShowConfirm(false);
    },
  });

  const handleFormSubmit = form.handleSubmit(() => {
    setShowConfirm(true);
  });

  const handleConfirm = () => {
    mutation.mutate(form.getValues());
  };

  const values = form.watch();

  return (
    <div
      className="rounded-[--radius-lg] border border-[--color-border-light] bg-[--color-bg-primary] p-5"
      data-testid="reconcile-mapping-tool"
    >
      <div className="flex items-center gap-3 mb-4 pb-3 border-b border-[--color-border-light]">
        <RefreshCw className="h-5 w-5 text-[--color-primary]" />
        <h2 className="text-sm font-semibold text-[--color-text-primary]">
          Reconcile Tenant Mapping
        </h2>
      </div>

      <p className="text-sm text-[--color-text-secondary] mb-4">
        Re-sync the tenant mapping for a specific tenant with the upstream registry.
      </p>

      <form onSubmit={handleFormSubmit} className="space-y-4">
        <FormInput
          label="Tenant ID"
          required
          placeholder="Enter the tenant ID to reconcile"
          {...form.register('tenant_id')}
          error={form.formState.errors.tenant_id?.message}
          data-testid="reconcile-tenant-id"
        />

        <FormTextarea
          label="Reason"
          required
          placeholder="Why is this reconciliation needed?"
          maxLength={500}
          showCharCount
          value={values.reason}
          {...form.register('reason')}
          error={form.formState.errors.reason?.message}
          data-testid="reconcile-reason"
        />

        <Button
          type="submit"
          variant="primary"
          size="sm"
          disabled={mutation.isPending}
          data-testid="reconcile-submit-btn"
        >
          Review & Reconcile
        </Button>
      </form>

      <ResultPanel
        result={mutation.data}
        error={mutation.error}
        onDismiss={() => mutation.reset()}
      />

      {/* Confirmation modal */}
      <Modal
        isOpen={showConfirm}
        title="Confirm: Reconcile Tenant Mapping"
        onClose={() => setShowConfirm(false)}
        size="sm"
      >
        <Modal.Body>
          <div className="space-y-3" data-testid="reconcile-confirm-summary">
            <p className="text-sm text-[--color-text-primary]">
              You are about to reconcile the tenant mapping.
            </p>
            <dl className="space-y-2 text-sm">
              <div>
                <dt className="font-medium text-[--color-text-secondary]">Tenant ID</dt>
                <dd className="text-[--color-text-primary]" data-testid="confirm-reconcile-tenant">
                  {values.tenant_id}
                </dd>
              </div>
              <div>
                <dt className="font-medium text-[--color-text-secondary]">Reason</dt>
                <dd className="text-[--color-text-primary]" data-testid="confirm-reconcile-reason">
                  {values.reason}
                </dd>
              </div>
            </dl>
          </div>
        </Modal.Body>
        <Modal.Actions>
          <Button
            variant="ghost"
            size="sm"
            onClick={() => setShowConfirm(false)}
            disableCooldown
          >
            Cancel
          </Button>
          <Button
            variant="primary"
            size="sm"
            loading={mutation.isPending}
            onClick={handleConfirm}
            data-testid="reconcile-confirm-btn"
          >
            Reconcile
          </Button>
        </Modal.Actions>
      </Modal>
    </div>
  );
}

// ── Page ────────────────────────────────────────────────────

export default function AdminToolsPage() {
  return (
    <div data-testid="admin-tools-page">
      <div className="mb-6">
        <h1 className="text-2xl font-bold text-[--color-text-primary]">Admin Tools</h1>
        <p className="text-sm text-[--color-text-secondary] mt-1">
          High-impact operations that require confirmation before execution.
        </p>
      </div>

      <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
        <RunBillingTool />
        <ReconcileMappingTool />
      </div>
    </div>
  );
}
