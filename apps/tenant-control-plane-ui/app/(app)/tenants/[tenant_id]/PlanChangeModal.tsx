// ============================================================
// PlanChangeModal — Change a tenant's plan with an effective date.
// Uses RHF + Zod for validation. Fetches active plans for select.
// Posts through BFF /api/tenants/[tenant_id]/plan-assignment.
// ============================================================
'use client';

import { useEffect } from 'react';
import { useForm } from 'react-hook-form';
import { zodResolver } from '@hookform/resolvers/zod';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { Button, Modal, FormSelect, FormInput } from '@/components/ui';
import { PlanAssignmentRequestSchema } from '@/lib/api/types';
import type { PlanAssignmentRequest, PlanListResponse } from '@/lib/api/types';

// ── API helpers ─────────────────────────────────────────────

async function fetchActivePlans(): Promise<PlanListResponse> {
  const res = await fetch('/api/plans?status=active&page_size=100');
  if (!res.ok) throw new Error(`HTTP ${res.status}`);
  return res.json();
}

async function assignPlan(tenantId: string, data: PlanAssignmentRequest): Promise<void> {
  const res = await fetch(
    `/api/tenants/${encodeURIComponent(tenantId)}/plan-assignment`,
    {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(data),
    },
  );
  if (!res.ok) {
    const body = await res.json().catch(() => ({ error: 'Unknown error' }));
    throw new Error(body.error ?? `HTTP ${res.status}`);
  }
}

// ── Today as YYYY-MM-DD ────────────────────────────────────

function todayString(): string {
  const d = new Date();
  const yyyy = d.getFullYear();
  const mm = String(d.getMonth() + 1).padStart(2, '0');
  const dd = String(d.getDate()).padStart(2, '0');
  return `${yyyy}-${mm}-${dd}`;
}

// ── Component ───────────────────────────────────────────────

interface PlanChangeModalProps {
  tenantId: string;
  currentPlan: string | undefined;
  isOpen: boolean;
  onClose: () => void;
}

export function PlanChangeModal({ tenantId, currentPlan, isOpen, onClose }: PlanChangeModalProps) {
  const queryClient = useQueryClient();

  const plansQuery = useQuery({
    queryKey: ['plans', 'active'],
    queryFn: fetchActivePlans,
    enabled: isOpen,
  });

  const {
    register,
    handleSubmit,
    reset,
    formState: { errors },
  } = useForm<PlanAssignmentRequest>({
    resolver: zodResolver(PlanAssignmentRequestSchema),
    defaultValues: { plan_id: '', effective_date: todayString() },
  });

  // Reset form when modal opens
  useEffect(() => {
    if (isOpen) {
      reset({ plan_id: '', effective_date: todayString() });
    }
  }, [isOpen, reset]);

  const mutation = useMutation({
    mutationFn: (data: PlanAssignmentRequest) => assignPlan(tenantId, data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['tenant', tenantId, 'plan-summary'] });
      queryClient.invalidateQueries({ queryKey: ['tenant', tenantId, 'detail'] });
      queryClient.invalidateQueries({ queryKey: ['tenant', tenantId, 'billing'] });
      queryClient.invalidateQueries({ queryKey: ['tenant', tenantId, 'features'] });
      onClose();
    },
  });

  const planOptions = (plansQuery.data?.plans ?? []).map((p) => ({
    value: p.id,
    label: p.name,
  }));

  const onSubmit = handleSubmit((data) => mutation.mutate(data));

  return (
    <Modal isOpen={isOpen} title="Change Plan" onClose={onClose} size="sm">
      <form onSubmit={onSubmit}>
        <Modal.Body>
          <div className="space-y-4">
            {currentPlan && (
              <p className="text-sm text-[--color-text-secondary]">
                Current plan: <strong>{currentPlan}</strong>
              </p>
            )}

            {plansQuery.isLoading ? (
              <p className="text-sm text-[--color-text-muted]">Loading plans...</p>
            ) : plansQuery.isError ? (
              <p className="text-sm text-[--color-danger]">Unable to load plans</p>
            ) : (
              <FormSelect
                label="New Plan"
                required
                placeholder="Select a plan..."
                options={planOptions}
                error={errors.plan_id?.message}
                data-testid="plan-select"
                {...register('plan_id')}
              />
            )}

            <FormInput
              label="Effective Date"
              type="date"
              required
              min={todayString()}
              error={errors.effective_date?.message}
              data-testid="effective-date-input"
              {...register('effective_date')}
            />

            {mutation.isError && (
              <p className="text-sm text-[--color-danger]" data-testid="plan-change-error">
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
          >
            Cancel
          </Button>
          <Button
            variant="primary"
            size="sm"
            type="submit"
            loading={mutation.isPending}
            disabled={plansQuery.isLoading || plansQuery.isError}
            data-testid="confirm-plan-change-btn"
          >
            Assign Plan
          </Button>
        </Modal.Actions>
      </form>
    </Modal>
  );
}
