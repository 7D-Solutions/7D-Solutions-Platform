"use client";

import Link from "next/link";
import { useSessionStore } from "@7d/platform-client";

export default function HomePage() {
  const claims = useSessionStore((s) => s.claims);

  return (
    <main className="flex min-h-screen flex-col items-center justify-center gap-6 px-4">
      <h1 className="text-4xl font-bold text-text-primary">__APP_TITLE__</h1>
      <p className="text-text-secondary">Powered by 7D Solutions Platform</p>

      {claims ? (
        <div className="flex flex-col items-center gap-3">
          <p className="text-success">
            Signed in as <span className="font-medium">{claims.sub}</span>
          </p>
          <LogoutButton />
        </div>
      ) : (
        <Link
          href="/login"
          className="rounded-md bg-primary px-5 py-2 text-sm font-medium text-text-inverse transition hover:bg-primary-dark"
        >
          Sign in
        </Link>
      )}
    </main>
  );
}

function LogoutButton() {
  const clearSession = useSessionStore((s) => s.clearSession);
  return (
    <button
      onClick={clearSession}
      className="rounded-md border border-border-default px-5 py-2 text-sm font-medium text-text-secondary transition hover:bg-bg-secondary"
    >
      Sign out
    </button>
  );
}
