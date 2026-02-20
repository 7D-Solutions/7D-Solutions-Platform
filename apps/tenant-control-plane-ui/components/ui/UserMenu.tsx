// ============================================================
// UserMenu — TopBar identity dropdown
// Shows staff email, role badge, and logout action.
// Fetches identity from /api/auth/me via TanStack Query.
// ============================================================
'use client';
import { useState, useRef, useEffect, useCallback } from 'react';
import { useQuery } from '@tanstack/react-query';
import { ChevronDown, LogOut, User } from 'lucide-react';
import { clsx } from 'clsx';

interface MeResponse {
  sub: string;
  email: string;
  roles: string[];
}

export function UserMenu() {
  const [open, setOpen] = useState(false);
  const menuRef = useRef<HTMLDivElement>(null);

  const { data: user } = useQuery<MeResponse>({
    queryKey: ['auth', 'me'],
    queryFn: async () => {
      const res = await fetch('/api/auth/me');
      if (!res.ok) throw new Error('Failed to fetch user');
      return res.json();
    },
    staleTime: 5 * 60 * 1000,
    retry: 1,
  });

  const handleLogout = useCallback(async () => {
    await fetch('/api/auth/logout', { method: 'POST' });
    window.location.href = '/login';
  }, []);

  // Close on outside click
  useEffect(() => {
    function onClickOutside(e: MouseEvent) {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    }
    if (open) {
      document.addEventListener('mousedown', onClickOutside);
      return () => document.removeEventListener('mousedown', onClickOutside);
    }
  }, [open]);

  // Close on Escape
  useEffect(() => {
    function onKeyDown(e: KeyboardEvent) {
      if (e.key === 'Escape') setOpen(false);
    }
    if (open) {
      document.addEventListener('keydown', onKeyDown);
      return () => document.removeEventListener('keydown', onKeyDown);
    }
  }, [open]);

  const displayEmail = user?.email ?? '…';
  const primaryRole = user?.roles?.includes('platform_admin') ? 'Platform Admin' : (user?.roles?.[0] ?? '');

  return (
    <div ref={menuRef} className="relative" data-testid="user-menu">
      <button
        type="button"
        onClick={() => setOpen((o) => !o)}
        aria-expanded={open}
        aria-haspopup="true"
        className={clsx(
          'flex items-center gap-2 rounded-[--radius-default] px-3 py-1.5 text-sm transition-[--transition-fast]',
          'text-[--color-text-secondary] hover:bg-[--color-bg-secondary] hover:text-[--color-text-primary]',
          open && 'bg-[--color-bg-secondary] text-[--color-text-primary]'
        )}
      >
        <User className="h-4 w-4" />
        <span className="max-w-[180px] truncate">{displayEmail}</span>
        {primaryRole && (
          <span className="rounded-full bg-blue-100 px-2 py-px text-[10px] font-medium text-blue-800">
            {primaryRole}
          </span>
        )}
        <ChevronDown className={clsx('h-3.5 w-3.5 transition-transform', open && 'rotate-180')} />
      </button>

      {open && (
        <div
          className="absolute right-0 top-full z-dropdown mt-1 w-56 rounded-[--radius-default] border border-[--color-border-light] bg-[--color-bg-primary] py-1 shadow-[--shadow-md]"
          role="menu"
        >
          <div className="border-b border-[--color-border-light] px-4 py-2">
            <p className="text-sm font-medium text-[--color-text-primary] truncate">{displayEmail}</p>
            {primaryRole && (
              <p className="text-xs text-[--color-text-muted]">{primaryRole}</p>
            )}
          </div>
          <button
            type="button"
            role="menuitem"
            onClick={handleLogout}
            className="flex w-full items-center gap-2 px-4 py-2 text-sm text-[--color-text-secondary] hover:bg-[--color-bg-secondary] hover:text-[--color-text-primary]"
          >
            <LogOut className="h-4 w-4" />
            Log out
          </button>
        </div>
      )}
    </div>
  );
}
