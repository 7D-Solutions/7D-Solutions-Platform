// ============================================================
// /login — Staff login page
// Server component wrapper with Suspense for useSearchParams.
// ============================================================
import { Suspense } from 'react';
import { LoginForm } from './LoginForm';

export default function LoginPage() {
  return (
    <div className="flex min-h-screen items-center justify-center bg-[--color-bg-secondary]">
      <div
        className="w-full max-w-sm rounded-[--radius-lg] border border-[--color-border-light] bg-[--color-bg-primary] p-8 shadow-[--shadow-md]"
      >
        <h1 className="mb-6 text-xl font-bold text-[--color-text-primary]">7D Platform — Staff Login</h1>
        <Suspense fallback={<div className="h-48" />}>
          <LoginForm />
        </Suspense>
      </div>
    </div>
  );
}
