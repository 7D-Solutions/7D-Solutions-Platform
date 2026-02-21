// ============================================================
// CreateTenantModal — 3-field form: name, plan, environment
// Posts to POST /api/tenants (BFF).
// Success: closes modal, invalidates tenants query, emits notification.
// Error: displays inline; if backend unavailable shows CLI fallback hint.
// ============================================================
'use client';

import { useEffect } from 'react';
import { useForm } from 'react-hook-form';
import { zodResolver } from '@hookform/resolvers/zod';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { Button, Modal, FormInput, FormSelect } from '@/components/ui';
import { CreateTenantRequestSchema } from '@/lib/api/types';
import type { CreateTenantRequest, PlanListResponse } from '@/lib/api/types';
import { useNotificationActions } from '@/infrastructure/state/notificationStore';

// ── Static options ──────────────────────────────────────────

const ENVIRONMENT_OPTIONS = [
  { value: 'development', label: 'Development' },
  { value: 'staging', label: 'Staging' },
  { value: 'production', label: 'Production' },
];

// ── API helpers ─────────────────────────────────────────────

async function fetchPlans(): Promise<PlanListResponse> {
  const res = await fetch('/api/plans?status=active&page_size=100');
  if (!res.ok) throw new Error(`HTTP ${res.status}`);
  return res.json();
}

async function postCreateTenant(body: CreateTenantRequest): Promise<unknown> {
  const res = await fetch('/api/tenants', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
  });
  const data = await res.json().catch(() => ({ error: 'Unknown error' }));
  if (!res.ok) throw new Error(data.error ?? `HTTP ${res.status}`);
  return data;
}

// ── Component ───────────────────────────────────────────────

interface CreateTenantModalProps {
  isOpen: boolean;
  onClose: () => void;
}

export function CreateTenantModal({ isOpen, onClose }: CreateTenantModalProps) {
  const queryClient = useQueryClient();
  const { addNotification } = useNotificationActions();

  const plansQuery = useQuery({
    queryKey: ['plans', 'active'],
    queryFn: fetchPlans,
    enabled: isOpen,
  });

  const {
    register,
    handleSubmit,
    reset,
    formState: { errors },
  } = useForm<CreateTenantRequest>({
    resolver: zodResolver(CreateTenantRequestSchema),
    defaultValues: { name: '', plan: '', environment: 'development' },
  });

  useEffect(() => {
    if (isOpen) reset({ name: '', plan: '', environment: 'development' });
  }, [isOpen, reset]);

  const mutation = useMutation({
    mutationFn: postCreateTenant,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['tenants'] });
      addNotification({ severity: 'success', title: 'Tenant created' });
      onClose();
    },
  });

  const planOptions = (plansQuery.data?.plans ?? []).map((p) => ({
    value: p.name,
    label: p.name,
  }));

  const onSubmit = handleSubmit((data) => mutation.mutate(data));

  return (
    <Modal isOpen={isOpen} title="New Tenant" onClose={onClose} size="sm">
      <div>
        <Modal.Body>
          <div className="space-y-4">
            <FormInput
              label="Name"
              required
              placeholder="Acme Corp"
              error={errors.name?.message}
              data-testid="create-tenant-name"
              {...register('name')}
            />

            {plansQuery.isLoading ? (
              <p className="text-sm text-[--color-text-muted]">Loading plans…</p>
            ) : (
              <FormSelect
                label="Plan"
                required
                options={planOptions}
                placeholder="Select a plan…"
                error={errors.plan?.message}
                data-testid="create-tenant-plan"
                {...register('plan')}
              />
            )}

            <FormSelect
              label="Environment"
              required
              options={ENVIRONMENT_OPTIONS}
              error={errors.environment?.message}
              data-testid="create-tenant-environment"
              {...register('environment')}
            />

            {mutation.isError && (
              <p
                className="text-sm text-[--color-danger]"
                role="alert"
                data-testid="create-tenant-error"
              >
                {mutation.error.message}
              </p>
            )}
          </div>
        </Modal.Body>

        <Modal.Actions>
          <Button
            variant="ghost"
            size="sm"
            type="button"
            onClick={onClose}
            disableCooldown
            data-testid="create-tenant-cancel"
          >
            Cancel
          </Button>
          <Button
            variant="primary"
            size="sm"
            type="button"
            loading={mutation.isPending}
            onClick={onSubmit}
            data-testid="create-tenant-submit"
          >
            Create Tenant
          </Button>
        </Modal.Actions>
      </div>
    </Modal>
  );
}
