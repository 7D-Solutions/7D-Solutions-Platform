// ============================================================
// App shell layout — top bar, left nav, content area
// All authenticated /app/** routes render inside this shell.
// ============================================================
'use client';
import Link from 'next/link';
import { usePathname } from 'next/navigation';
import { clsx } from 'clsx';
import { Users, CreditCard, Settings, BarChart2, Shield, LogOut, Package, Boxes, Key, ClipboardList, Activity, ShieldAlert } from 'lucide-react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { Button } from '@/components/ui/Button';
import { NotificationCenter } from '@/components/ui/NotificationCenter';
import { UserMenu } from '@/components/ui/UserMenu';
import { IdleWarningModal } from '@/components/ui/IdleWarningModal';
import { TabBar } from '@/components/ui/TabBar';
import { useIdleTimeout } from '@/infrastructure/hooks/useIdleTimeout';
import { useSplitView } from '@/infrastructure/state/tabStore';
import { SUPPORT_SESSION_POLL_MS } from '@/lib/constants';
import { useState, useCallback } from 'react';

const navItems = [
  { label: 'Tenants',          href: '/tenants',  icon: Users       },
  { label: 'Plans & Pricing',  href: '/plans',    icon: Package     },
  { label: 'Bundles',          href: '/bundles',       icon: Boxes       },
  { label: 'Entitlements',     href: '/entitlements',  icon: Key         },
  { label: 'Billing',          href: '/billing',       icon: CreditCard  },
  { label: 'Audit Log',         href: '/audit',    icon: ClipboardList },
  { label: 'System Status',    href: '/system/status', icon: Activity },
  { label: 'Reports',          href: '/reports',  icon: BarChart2   },
  { label: 'IAM',              href: '/iam',      icon: Shield      },
  { label: 'Settings',         href: '/settings', icon: Settings    },
];

interface MeResponse {
  sub: string;
  email: string;
  roles: string[];
  actor_type: 'staff' | 'support';
  support_tenant_id?: string;
}

async function logout() {
  await fetch('/api/auth/logout', { method: 'POST' });
  window.location.href = '/login';
}

export default function AppLayout({ children }: { children: React.ReactNode }) {
  const pathname = usePathname();
  const splitView = useSplitView();
  const queryClient = useQueryClient();
  const [showIdleWarning, setShowIdleWarning] = useState(false);

  // Poll for support session status
  const meQuery = useQuery<MeResponse>({
    queryKey: ['auth', 'me'],
    queryFn: async () => {
      const res = await fetch('/api/auth/me');
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      return res.json();
    },
    refetchInterval: SUPPORT_SESSION_POLL_MS,
    staleTime: 10_000,
  });

  const isSupportSession = meQuery.data?.actor_type === 'support';
  const supportTenantId = meQuery.data?.support_tenant_id;

  const endSupportMutation = useMutation({
    mutationFn: async () => {
      if (!supportTenantId) return;
      const res = await fetch(
        `/api/tenants/${encodeURIComponent(supportTenantId)}/support-sessions/end`,
        { method: 'POST' },
      );
      if (!res.ok) throw new Error('Failed to end session');
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['auth', 'me'] });
    },
  });

  // Stable callback refs — prevent useIdleTimeout effects from re-running
  const handleIdleWarning = useCallback(() => setShowIdleWarning(true), []);
  const handleIdleTimeout = useCallback(() => {
    setShowIdleWarning(false);
    logout();
  }, []);

  const { remainingMs, resetTimer } = useIdleTimeout({
    onWarning: handleIdleWarning,
    onTimeout: handleIdleTimeout,
    enabled: true,
  });

  const handleStayLoggedIn = useCallback(() => {
    setShowIdleWarning(false);
    resetTimer();
  }, [resetTimer]);

  return (
    <>
      <div className="flex h-screen bg-[--color-bg-secondary]">
        {/* Left nav */}
        <aside
          className="flex flex-col bg-[--color-bg-primary] border-r border-[--color-border-light]"
          style={{ width: 'var(--sidebar-width)' }}
        >
          {/* Logo */}
          <div
            className="flex items-center px-6 border-b border-[--color-border-light]"
            style={{ height: 'var(--header-height)' }}
          >
            <span className="text-lg font-bold text-[--color-primary]">7D TCP</span>
          </div>

          {/* Nav items */}
          <nav className="flex-1 overflow-y-auto py-4">
            {navItems.map(({ label, href, icon: Icon }) => {
              const isActive = pathname.startsWith(href);
              return (
                <Link
                  key={href}
                  href={href}
                  className={clsx(
                    'flex items-center gap-3 px-6 py-2.5 text-sm transition-[--transition-fast]',
                    isActive
                      ? 'bg-[--color-primary] text-[--color-text-inverse] font-medium'
                      : 'text-[--color-text-secondary] hover:bg-[--color-bg-secondary] hover:text-[--color-text-primary]'
                  )}
                >
                  <Icon className="h-4 w-4 flex-shrink-0" />
                  {label}
                </Link>
              );
            })}
          </nav>

          {/* Logout */}
          <div className="border-t border-[--color-border-light] p-4">
            <Button
              variant="ghost"
              size="sm"
              onClick={logout}
              className="w-full justify-start"
              icon={LogOut}
              iconPosition="left"
            >
              Log out
            </Button>
          </div>
        </aside>

        {/* Main content */}
        <div className="flex flex-1 flex-col overflow-hidden">
          {/* Support session banner */}
          {isSupportSession && (
            <div
              className="flex items-center justify-between gap-3 border-b border-amber-400 bg-amber-50 px-6 py-2"
              data-testid="support-session-banner"
            >
              <div className="flex items-center gap-2">
                <ShieldAlert className="h-4 w-4 text-amber-600" />
                <span className="text-sm font-medium text-amber-800">
                  Support session active
                  {supportTenantId && (
                    <span className="font-normal text-amber-700">
                      {' '}— tenant {supportTenantId}
                    </span>
                  )}
                </span>
              </div>
              <Button
                variant="warning"
                size="xs"
                loading={endSupportMutation.isPending}
                onClick={() => endSupportMutation.mutate()}
                data-testid="banner-end-session-btn"
              >
                End Session
              </Button>
            </div>
          )}

          {/* Top bar */}
          <header
            className="flex items-center justify-end gap-3 border-b border-[--color-border-light] bg-[--color-bg-primary] px-6"
            style={{ height: 'var(--header-height)' }}
          >
            <NotificationCenter />
            <UserMenu />
          </header>

          {/* Tab bar */}
          <TabBar />

          {/* Page content — split view or single pane */}
          {splitView.enabled ? (
            <div className="flex flex-1 overflow-hidden">
              <div
                className="overflow-y-auto p-6"
                style={{ width: `${splitView.dividerPosition}%` }}
                data-testid="split-left"
              >
                {children}
              </div>
              <div
                className="w-1 cursor-col-resize bg-[--color-border-default] hover:bg-[--color-primary] transition-[--transition-fast]"
                data-testid="split-divider"
              />
              <div
                className="overflow-y-auto p-6"
                style={{ width: `${100 - splitView.dividerPosition}%` }}
                data-testid="split-right"
              >
                {children}
              </div>
            </div>
          ) : (
            <main className="flex-1 overflow-y-auto p-6">
              {children}
            </main>
          )}
        </div>
      </div>

      <IdleWarningModal
        isOpen={showIdleWarning}
        remainingMs={remainingMs}
        onStayLoggedIn={handleStayLoggedIn}
        onLogout={logout}
      />
    </>
  );
}
