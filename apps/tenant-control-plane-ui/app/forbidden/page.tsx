// ============================================================
// /forbidden — Access denied page
// Shown when a user authenticates but lacks platform_admin role.
// ============================================================
'use client';
import { ShieldX } from 'lucide-react';
import { Button } from '@/components/ui/Button';

async function handleLogout() {
  await fetch('/api/auth/logout', { method: 'POST' });
  window.location.href = '/login';
}

export default function ForbiddenPage() {
  return (
    <div className="flex min-h-screen items-center justify-center bg-[--color-bg-secondary]">
      <div className="w-full max-w-md rounded-[--radius-lg] border border-[--color-border-light] bg-[--color-bg-primary] p-8 text-center shadow-[--shadow-md]">
        <ShieldX className="mx-auto mb-4 h-12 w-12 text-[--color-danger]" />
        <h1 className="mb-2 text-xl font-bold text-[--color-text-primary]">Access Denied</h1>
        <p className="mb-6 text-sm text-[--color-text-secondary]">
          Your account does not have the <strong>platform_admin</strong> role required
          to access the Tenant Control Plane. Contact your administrator if you believe
          this is an error.
        </p>
        <Button variant="primary" size="md" onClick={handleLogout}>
          Sign in with a different account
        </Button>
      </div>
    </div>
  );
}
