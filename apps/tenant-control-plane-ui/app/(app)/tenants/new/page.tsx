// ============================================================
// /app/tenants/new — Self-serve tenant onboarding wizard
// Step 1: Tenant details (name, environment)
// Step 2: Plan selection (from plan catalog)
// Step 3: Initial admin user (email, password)
// All browser calls go through BFF routes — never direct to Rust.
// Auth: platform_admin required (enforced in middleware + BFF routes).
// ============================================================
'use client';
import { useState, useCallback } from 'react';
import { useRouter } from 'next/navigation';
import { useForm } from 'react-hook-form';
import { zodResolver } from '@hookform/resolvers/zod';
import { useQuery } from '@tanstack/react-query';
import { z } from 'zod';
import { CheckCircle, ChevronRight, ArrowLeft } from 'lucide-react';
import Link from 'next/link';
import { Button, FormInput, FormSelect } from '@/components/ui';
import { useNotificationActions } from '@/infrastructure/state/notificationStore';
import type { PlanListResponse } from '@/lib/api/types';

// ── Local schemas ───────────────────────────────────────────

const Step1Schema = z.object({
  name: z.string().min(1, 'Name is required').max(100, 'Name must be 100 characters or fewer'),
  environment: z.enum(['development', 'staging', 'production'], {
    errorMap: () => ({ message: 'Environment is required' }),
  }),
});

const Step3Schema = z
  .object({
    email: z.string().email('Valid email required'),
    password: z.string().min(8, 'Password must be at least 8 characters').max(128),
    confirm_password: z.string(),
  })
  .refine((d) => d.password === d.confirm_password, {
    message: 'Passwords do not match',
    path: ['confirm_password'],
  });

type Step1Data = z.infer<typeof Step1Schema>;
type Step3Data = z.infer<typeof Step3Schema>;

// ── Static options ──────────────────────────────────────────

const ENVIRONMENT_OPTIONS = [
  { value: 'development', label: 'Development' },
  { value: 'staging', label: 'Staging' },
  { value: 'production', label: 'Production' },
];

const STEP_LABELS = ['Tenant details', 'Plan', 'Admin user'];

// ── API helpers ─────────────────────────────────────────────

async function fetchPlans(): Promise<PlanListResponse> {
  const res = await fetch('/api/plans?status=active&page_size=50');
  if (!res.ok) throw new Error(`HTTP ${res.status}`);
  return res.json();
}

async function postCreateTenant(body: {
  name: string;
  plan: string;
  environment: string;
}): Promise<{ id: string }> {
  const res = await fetch('/api/tenants', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
  });
  const data = await res.json().catch(() => ({ error: 'Unknown error' }));
  if (!res.ok) throw new Error(data.error ?? `HTTP ${res.status}`);
  return data;
}

async function postCreateUser(
  tenantId: string,
  body: { email: string; password: string },
): Promise<{ id: string; email: string }> {
  const res = await fetch(`/api/tenants/${encodeURIComponent(tenantId)}/users`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
  });
  const data = await res.json().catch(() => ({ error: 'Unknown error' }));
  if (!res.ok) throw new Error(data.error ?? `HTTP ${res.status}`);
  return data;
}

// ── Step indicator ──────────────────────────────────────────

function StepIndicator({ current, total }: { current: number; total: number }) {
  return (
    <nav aria-label="Wizard steps" className="flex items-center gap-0 mb-8">
      {STEP_LABELS.map((label, i) => {
        const step = i + 1;
        const done = step < current;
        const active = step === current;
        return (
          <div key={step} className="flex items-center">
            <div className="flex items-center gap-2">
              <div
                className={[
                  'flex h-7 w-7 items-center justify-center rounded-full text-xs font-semibold border',
                  done
                    ? 'bg-[--color-primary] border-[--color-primary] text-white'
                    : active
                      ? 'border-[--color-primary] text-[--color-primary] bg-white'
                      : 'border-[--color-border-default] text-[--color-text-muted] bg-white',
                ].join(' ')}
                aria-current={active ? 'step' : undefined}
              >
                {done ? <CheckCircle className="h-4 w-4" /> : step}
              </div>
              <span
                className={[
                  'text-sm',
                  active ? 'font-semibold text-[--color-text-primary]' : 'text-[--color-text-muted]',
                ].join(' ')}
              >
                {label}
              </span>
            </div>
            {step < total && (
              <ChevronRight className="h-4 w-4 mx-2 text-[--color-text-muted]" />
            )}
          </div>
        );
      })}
    </nav>
  );
}

// ── Step 1: Tenant details ──────────────────────────────────

function Step1Form({
  defaultValues,
  onNext,
}: {
  defaultValues?: Partial<Step1Data>;
  onNext: (data: Step1Data) => void;
}) {
  const { register, handleSubmit, formState: { errors } } = useForm<Step1Data>({
    resolver: zodResolver(Step1Schema),
    defaultValues: defaultValues ?? { name: '', environment: 'development' },
  });
  return (
    // onSubmit kept for keyboard Enter support; button uses type="button" to avoid
    // Button cooldown disabling the element before the browser fires submit.
    <form onSubmit={handleSubmit(onNext)} className="space-y-4" data-testid="wizard-step-1">
      <FormInput
        label="Tenant name"
        required
        placeholder="Acme Corp"
        error={errors.name?.message}
        data-testid="wizard-name"
        {...register('name')}
      />
      <FormSelect
        label="Environment"
        required
        options={ENVIRONMENT_OPTIONS}
        error={errors.environment?.message}
        data-testid="wizard-environment"
        {...register('environment')}
      />
      <div className="flex justify-end pt-2">
        <Button
          variant="primary"
          size="sm"
          type="button"
          onClick={handleSubmit(onNext)}
          data-testid="wizard-next"
        >
          Next
        </Button>
      </div>
    </form>
  );
}

// ── Step 2: Plan selection ──────────────────────────────────

function Step2Form({
  selectedPlan,
  onNext,
  onBack,
}: {
  selectedPlan: string;
  onNext: (plan: string) => void;
  onBack: () => void;
}) {
  const [plan, setPlan] = useState(selectedPlan);
  const [error, setError] = useState('');
  const plansQuery = useQuery({ queryKey: ['plans', 'active'], queryFn: fetchPlans });

  const handleNext = () => {
    if (!plan) { setError('Select a plan to continue'); return; }
    setError('');
    onNext(plan);
  };

  const plans = plansQuery.data?.plans ?? [];

  return (
    <div data-testid="wizard-step-2">
      {plansQuery.isLoading && (
        <p className="text-sm text-[--color-text-muted] mb-4">Loading plans…</p>
      )}
      {plansQuery.isError && (
        <p className="text-sm text-[--color-danger] mb-4">
          Could not load plans. Check that the plan catalog service is available.
        </p>
      )}
      <div className="space-y-2 mb-4">
        {plans.map((p) => (
          // disableCooldown: plan cards are selection-only toggles, not action buttons
          <Button
            key={p.id}
            type="button"
            disableCooldown
            onClick={() => { setPlan(p.name); setError(''); }}
            data-testid="wizard-plan-option"
            aria-pressed={plan === p.name}
            className={[
              '!w-full !justify-start !text-left !p-4 !rounded-[--radius-lg] !min-h-0 !font-normal',
              plan === p.name
                ? '!border-[--color-primary] !bg-[--color-primary]/5 !text-[--color-text-primary]'
                : '!border-[--color-border-default] !bg-[--color-bg-primary] !text-[--color-text-primary] hover:!border-[--color-primary]/50 !bg-transparent',
            ].join(' ')}
          >
            <div className="w-full">
              <div className="flex items-center justify-between">
                <span className="font-semibold text-[--color-text-primary]">{p.name}</span>
                <span className="text-xs text-[--color-text-muted] capitalize">{p.pricing_model}</span>
              </div>
              <p className="text-sm text-[--color-text-secondary] mt-1">
                {p.included_seats} seat{p.included_seats !== 1 ? 's' : ''} included
              </p>
            </div>
          </Button>
        ))}
        {!plansQuery.isLoading && plans.length === 0 && (
          <p className="text-sm text-[--color-text-muted]">No active plans found.</p>
        )}
      </div>
      {error && (
        <p className="text-sm text-[--color-danger] mb-3" role="alert">{error}</p>
      )}
      <div className="flex justify-between">
        <Button variant="ghost" size="sm" type="button" onClick={onBack}>
          <ArrowLeft className="h-4 w-4 mr-1" /> Back
        </Button>
        <Button
          variant="primary"
          size="sm"
          type="button"
          onClick={handleNext}
          data-testid="wizard-next"
          disabled={!plan}
        >
          Next
        </Button>
      </div>
    </div>
  );
}

// ── Step 3: Admin user ──────────────────────────────────────

function Step3Form({
  defaultValues,
  submitting,
  submitError,
  onSubmit,
  onBack,
}: {
  defaultValues?: Partial<Step3Data>;
  submitting: boolean;
  submitError: string;
  onSubmit: (data: Step3Data) => void;
  onBack: () => void;
}) {
  const { register, handleSubmit, formState: { errors } } = useForm<Step3Data>({
    resolver: zodResolver(Step3Schema),
    defaultValues: defaultValues ?? { email: '', password: '', confirm_password: '' },
  });
  return (
    // onSubmit kept for keyboard Enter support; button uses type="button" to avoid
    // Button cooldown disabling the element before the browser fires submit.
    <form onSubmit={handleSubmit(onSubmit)} className="space-y-4" data-testid="wizard-step-3">
      <p className="text-sm text-[--color-text-secondary]">
        Create the initial administrator account for this tenant. They can add more users after logging in.
      </p>
      <FormInput
        label="Admin email"
        required
        type="email"
        placeholder="admin@tenant.com"
        error={errors.email?.message}
        data-testid="wizard-email"
        {...register('email')}
      />
      <FormInput
        label="Password"
        required
        type="password"
        placeholder="Minimum 8 characters"
        error={errors.password?.message}
        data-testid="wizard-password"
        {...register('password')}
      />
      <FormInput
        label="Confirm password"
        required
        type="password"
        placeholder="Repeat password"
        error={errors.confirm_password?.message}
        data-testid="wizard-confirm-password"
        {...register('confirm_password')}
      />
      {submitError && (
        <p className="text-sm text-[--color-danger]" role="alert" data-testid="wizard-error">
          {submitError}
        </p>
      )}
      <div className="flex justify-between pt-2">
        <Button variant="ghost" size="sm" type="button" onClick={onBack} disabled={submitting}>
          <ArrowLeft className="h-4 w-4 mr-1" /> Back
        </Button>
        <Button
          variant="primary"
          size="sm"
          type="button"
          onClick={handleSubmit(onSubmit)}
          loading={submitting}
          data-testid="wizard-submit"
        >
          Create Tenant
        </Button>
      </div>
    </form>
  );
}

// ── Success screen ──────────────────────────────────────────

function SuccessScreen({ tenantId, tenantName }: { tenantId: string; tenantName: string }) {
  return (
    <div className="text-center py-6" data-testid="wizard-success">
      <CheckCircle className="h-12 w-12 text-[--color-success] mx-auto mb-4" />
      <h2 className="text-lg font-semibold text-[--color-text-primary] mb-2">
        Tenant created
      </h2>
      <p className="text-sm text-[--color-text-secondary] mb-6">
        <strong>{tenantName}</strong> is ready. The admin user has been provisioned and can log in.
      </p>
      <div className="flex flex-col gap-3 items-center">
        <Link
          href={`/tenants/${tenantId}`}
          className="inline-flex items-center gap-1 text-sm font-medium text-[--color-primary] hover:underline"
          data-testid="wizard-goto-tenant"
        >
          Go to tenant <ChevronRight className="h-4 w-4" />
        </Link>
        <Link
          href="/tenants"
          className="text-sm text-[--color-text-muted] hover:underline"
        >
          Back to tenant list
        </Link>
      </div>
    </div>
  );
}

// ── Main wizard page ────────────────────────────────────────

export default function NewTenantPage() {
  const router = useRouter();
  const { addNotification } = useNotificationActions();

  const [step, setStep] = useState(1);
  const [step1Data, setStep1Data] = useState<Step1Data | null>(null);
  const [chosenPlan, setChosenPlan] = useState('');
  const [submitting, setSubmitting] = useState(false);
  const [submitError, setSubmitError] = useState('');
  const [createdTenantId, setCreatedTenantId] = useState('');
  const [createdTenantName, setCreatedTenantName] = useState('');

  const handleStep1 = useCallback((data: Step1Data) => {
    setStep1Data(data);
    setStep(2);
  }, []);

  const handleStep2 = useCallback((plan: string) => {
    setChosenPlan(plan);
    setStep(3);
  }, []);

  const handleStep3 = useCallback(
    async (data: Step3Data) => {
      if (!step1Data) return;
      setSubmitting(true);
      setSubmitError('');
      try {
        const tenant = await postCreateTenant({
          name: step1Data.name,
          plan: chosenPlan,
          environment: step1Data.environment,
        });
        const tenantId = tenant.id;
        try {
          await postCreateUser(tenantId, { email: data.email, password: data.password });
        } catch (userErr) {
          // Tenant was created but user creation failed — partial success
          addNotification({
            severity: 'warning',
            title: 'Tenant created — user provisioning failed',
            message: userErr instanceof Error ? userErr.message : 'Use tenantctl to add the admin user.',
          });
          router.push(`/tenants/${tenantId}`);
          return;
        }
        setCreatedTenantId(tenantId);
        setCreatedTenantName(step1Data.name);
        addNotification({ severity: 'success', title: 'Tenant and admin user created' });
        setStep(4);
      } catch (err) {
        setSubmitError(err instanceof Error ? err.message : 'Failed to create tenant');
      } finally {
        setSubmitting(false);
      }
    },
    [step1Data, chosenPlan, addNotification, router],
  );

  return (
    <div className="max-w-xl mx-auto py-8 px-4">
      {/* Back link */}
      {step < 4 && (
        <Link
          href="/tenants"
          className="inline-flex items-center gap-1 text-sm text-[--color-text-muted] hover:text-[--color-text-primary] mb-6"
        >
          <ArrowLeft className="h-4 w-4" /> All tenants
        </Link>
      )}

      <div className="rounded-[--radius-xl] border border-[--color-border-light] bg-[--color-bg-primary] p-8 shadow-sm">
        {step < 4 && (
          <>
            <h1 className="text-xl font-semibold text-[--color-text-primary] mb-1">
              New tenant
            </h1>
            <p className="text-sm text-[--color-text-secondary] mb-6">
              Provision a tenant, assign a plan, and create its first admin account.
            </p>
            <StepIndicator current={step} total={3} />
          </>
        )}

        {step === 1 && (
          <Step1Form defaultValues={step1Data ?? undefined} onNext={handleStep1} />
        )}
        {step === 2 && (
          <Step2Form
            selectedPlan={chosenPlan}
            onNext={handleStep2}
            onBack={() => setStep(1)}
          />
        )}
        {step === 3 && (
          <Step3Form
            submitting={submitting}
            submitError={submitError}
            onSubmit={handleStep3}
            onBack={() => setStep(2)}
          />
        )}
        {step === 4 && (
          <SuccessScreen tenantId={createdTenantId} tenantName={createdTenantName} />
        )}
      </div>
    </div>
  );
}
