// ============================================================
// LoginForm — client component handling RHF+Zod login
// Extracted so the parent page.tsx can wrap it in Suspense
// (required by Next.js for useSearchParams).
// ============================================================
'use client';
import { useState, useRef } from 'react';
import { useRouter, useSearchParams } from 'next/navigation';
import { useForm } from 'react-hook-form';
import { zodResolver } from '@hookform/resolvers/zod';
import { z } from 'zod';
import { FormInput } from '@/components/ui/FormInput';
import { Button } from '@/components/ui/Button';
import { BUTTON_COOLDOWN_MS } from '@/lib/constants';

const loginSchema = z.object({
  email: z.string().min(1, 'Email is required').email('Enter a valid email address'),
  password: z.string().min(1, 'Password is required'),
});

type LoginFormData = z.infer<typeof loginSchema>;

const REASON_MESSAGES: Record<string, string> = {
  expired: 'Your session has expired. Please sign in again.',
  forbidden: 'You do not have access to this resource.',
};

export function LoginForm() {
  const router = useRouter();
  const searchParams = useSearchParams();
  const [serverError, setServerError] = useState('');
  const [loading, setLoading] = useState(false);
  const cooldownRef = useRef(false);

  const reason = searchParams.get('reason');
  const redirectTo = searchParams.get('redirect');
  const reasonMessage = reason ? REASON_MESSAGES[reason] : null;

  const {
    register,
    handleSubmit,
    formState: { errors },
  } = useForm<LoginFormData>({
    resolver: zodResolver(loginSchema),
    defaultValues: { email: '', password: '' },
  });

  async function onSubmit(data: LoginFormData) {
    if (cooldownRef.current || loading) return;
    cooldownRef.current = true;
    setTimeout(() => { cooldownRef.current = false; }, BUTTON_COOLDOWN_MS);

    setServerError('');
    setLoading(true);
    try {
      const res = await fetch('/api/auth/login', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(data),
      });
      if (res.ok) {
        router.replace(redirectTo ?? '/tenants');
      } else {
        const body = await res.json().catch(() => ({}));
        setServerError(body.error ?? 'Login failed. Please try again.');
      }
    } catch {
      setServerError('Network error. Please try again.');
    } finally {
      setLoading(false);
    }
  }

  return (
    <>
      {reasonMessage && (
        <p className="mb-4 rounded-[--radius-default] bg-amber-50 px-3 py-2 text-sm text-amber-700">
          {reasonMessage}
        </p>
      )}

      <form onSubmit={handleSubmit(onSubmit)} noValidate>
        <div className="mb-4">
          <FormInput
            label="Email"
            type="email"
            autoComplete="email"
            required
            placeholder="staff@7dsolutions.com"
            error={errors.email?.message}
            {...register('email')}
          />
        </div>

        <div className="mb-6">
          <FormInput
            label="Password"
            type="password"
            autoComplete="current-password"
            required
            error={errors.password?.message}
            {...register('password')}
          />
        </div>

        {serverError && (
          <p data-testid="server-error" className="mb-4 rounded-[--radius-default] bg-red-50 px-3 py-2 text-sm text-red-700">
            {serverError}
          </p>
        )}

        <Button
          type="submit"
          variant="primary"
          size="md"
          loading={loading}
          className="w-full"
          // disableCooldown: login form already guards double-submit via cooldownRef
          disableCooldown
        >
          {loading ? 'Signing in…' : 'Sign in'}
        </Button>
      </form>
    </>
  );
}
