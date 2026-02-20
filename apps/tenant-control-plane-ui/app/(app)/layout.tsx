// ============================================================
// App shell layout — top bar, left nav, content area
// All authenticated /app/** routes render inside this shell.
// ============================================================
'use client';
import Link from 'next/link';
import { usePathname } from 'next/navigation';
import { clsx } from 'clsx';
import { Users, CreditCard, Settings, BarChart2, Shield, LogOut, Package } from 'lucide-react';
import { Button } from '@/components/ui/Button';
import { NotificationCenter } from '@/components/ui/NotificationCenter';
import { IdleWarningModal } from '@/components/ui/IdleWarningModal';
import { TabBar } from '@/components/ui/TabBar';
import { useIdleTimeout } from '@/infrastructure/hooks/useIdleTimeout';
import { useSplitView, useTabActions, useActiveTabId } from '@/infrastructure/state/tabStore';
import { useState, useCallback } from 'react';

const navItems = [
  { label: 'Tenants',          href: '/tenants',  icon: Users       },
  { label: 'Plans & Pricing',  href: '/plans',    icon: Package     },
  { label: 'Billing',          href: '/billing',  icon: CreditCard  },
  { label: 'Reports',          href: '/reports',  icon: BarChart2   },
  { label: 'IAM',              href: '/iam',      icon: Shield      },
  { label: 'Settings',         href: '/settings', icon: Settings    },
];

async function logout() {
  await fetch('/api/auth/logout', { method: 'POST' });
  window.location.href = '/login';
}

export default function AppLayout({ children }: { children: React.ReactNode }) {
  const pathname = usePathname();
  const splitView = useSplitView();
  const { getTab, setDividerPosition } = useTabActions();
  const activeTabId = useActiveTabId();
  const [showIdleWarning, setShowIdleWarning] = useState(false);

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
          {/* Top bar */}
          <header
            className="flex items-center justify-end gap-3 border-b border-[--color-border-light] bg-[--color-bg-primary] px-6"
            style={{ height: 'var(--header-height)' }}
          >
            <NotificationCenter />
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
