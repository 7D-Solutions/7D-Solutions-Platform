// ============================================================
// /app/login — Staff login page
// Submits to BFF /api/auth/login which sets httpOnly cookie.
// ============================================================
'use client';
import { useState, useRef } from 'react';
import { useRouter } from 'next/navigation';
import { Button } from '@/components/ui/Button';
import { BUTTON_COOLDOWN_MS } from '@/lib/constants';

export default function LoginPage() {
  const router = useRouter();
  const [email, setEmail] = useState('');
  const [password, setPassword] = useState('');
  const [error, setError] = useState('');
  const [loading, setLoading] = useState(false);
  const cooldownRef = useRef(false);

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (cooldownRef.current || loading) return;
    cooldownRef.current = true;
    setTimeout(() => { cooldownRef.current = false; }, BUTTON_COOLDOWN_MS);

    setError('');
    setLoading(true);
    try {
      const res = await fetch('/api/auth/login', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ email, password }),
      });
      if (res.ok) {
        router.replace('/app/tenants');
      } else {
        const data = await res.json().catch(() => ({}));
        setError(data.error ?? 'Login failed. Please try again.');
      }
    } catch {
      setError('Network error. Please try again.');
    } finally {
      setLoading(false);
    }
  }

  return (
    <div className="flex min-h-screen items-center justify-center bg-[--color-bg-secondary]">
      <div
        className="w-full max-w-sm rounded-[--radius-lg] border border-[--color-border-light] bg-[--color-bg-primary] p-8 shadow-[--shadow-md]"
      >
        <h1 className="mb-6 text-xl font-bold text-[--color-text-primary]">7D Platform — Staff Login</h1>

        <form onSubmit={handleSubmit} noValidate>
          <div className="mb-4">
            <label
              htmlFor="email"
              className="mb-1 block text-sm font-medium text-[--color-text-primary]"
            >
              Email
            </label>
            <input
              id="email"
              type="email"
              autoComplete="email"
              required
              value={email}
              onChange={(e) => setEmail(e.target.value)}
              className="block w-full rounded-[--radius-default] border border-[--color-border-default] bg-[--color-bg-primary] px-3 py-2 text-sm text-[--color-text-primary] placeholder-[--color-text-muted] focus:border-[--color-primary] focus:outline-none focus:ring-2 focus:ring-[--color-primary-faint]"
              placeholder="staff@7dsolutions.com"
            />
          </div>

          <div className="mb-6">
            <label
              htmlFor="password"
              className="mb-1 block text-sm font-medium text-[--color-text-primary]"
            >
              Password
            </label>
            <input
              id="password"
              type="password"
              autoComplete="current-password"
              required
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              className="block w-full rounded-[--radius-default] border border-[--color-border-default] bg-[--color-bg-primary] px-3 py-2 text-sm text-[--color-text-primary] placeholder-[--color-text-muted] focus:border-[--color-primary] focus:outline-none focus:ring-2 focus:ring-[--color-primary-faint]"
            />
          </div>

          {error && (
            <p className="mb-4 rounded-[--radius-default] bg-red-50 px-3 py-2 text-sm text-red-700">
              {error}
            </p>
          )}

          <Button
            type="submit"
            variant="primary"
            size="md"
            loading={loading}
            disabled={!email || !password}
            className="w-full"
            // disableCooldown: login form already guards double-submit via cooldownRef
            disableCooldown
          >
            {loading ? 'Signing in…' : 'Sign in'}
          </Button>
        </form>
      </div>
    </div>
  );
}
