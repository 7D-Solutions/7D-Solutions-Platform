'use client';
// ============================================================
// NotificationCenter — bell icon + badge + notification dropdown
// ============================================================
import { useState, useRef, useEffect } from 'react';
import { clsx } from 'clsx';
import { Bell, CheckCheck, Trash2 } from 'lucide-react';
import {
  useNotifications,
  useUnreadCount,
  useNotificationActions,
} from '@/infrastructure/state/notificationStore';
import { NotificationItem } from './NotificationItem';

export function NotificationCenter() {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  const notifications = useNotifications();
  const unreadCount = useUnreadCount();
  const { markAsRead, markAllAsRead, dismissNotification, clearAll } = useNotificationActions();

  useEffect(() => {
    const handleClickOutside = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    document.addEventListener('mousedown', handleClickOutside);
    return () => document.removeEventListener('mousedown', handleClickOutside);
  }, []);

  return (
    <div ref={ref} className="relative">
      <button
        onClick={() => setOpen(!open)}
        className="relative rounded-[--radius-default] p-2 text-[--color-text-secondary] hover:bg-[--color-bg-tertiary] transition-[--transition-fast]"
        aria-label={`Notifications${unreadCount > 0 ? `, ${unreadCount} unread` : ''}`}
      >
        <Bell className="h-5 w-5" />
        {unreadCount > 0 && (
          <span
            className="absolute -right-0.5 -top-0.5 flex h-4 w-4 items-center justify-center rounded-full bg-[--color-danger] text-[10px] font-bold text-white"
          >
            {unreadCount > 99 ? '99+' : unreadCount}
          </span>
        )}
      </button>

      {open && (
        <div
          className="absolute right-0 mt-1 w-80 rounded-[--radius-lg] border border-[--color-border-light] bg-[--color-bg-primary] shadow-[--shadow-xl]"
          style={{ zIndex: 'var(--z-dropdown)' }}
        >
          {/* Header */}
          <div className="flex items-center justify-between border-b border-[--color-border-light] px-4 py-3">
            <h3 className="text-sm font-semibold text-[--color-text-primary]">
              Notifications
              {unreadCount > 0 && (
                <span className="ml-2 rounded-full bg-[--color-danger] px-1.5 py-0.5 text-[10px] text-white">
                  {unreadCount}
                </span>
              )}
            </h3>
            <div className="flex items-center gap-1">
              {unreadCount > 0 && (
                <button
                  onClick={markAllAsRead}
                  title="Mark all as read"
                  className="rounded p-1 text-[--color-text-secondary] hover:bg-[--color-bg-tertiary]"
                >
                  <CheckCheck className="h-4 w-4" />
                </button>
              )}
              {notifications.length > 0 && (
                <button
                  onClick={clearAll}
                  title="Clear all"
                  className="rounded p-1 text-[--color-text-secondary] hover:bg-[--color-bg-tertiary]"
                >
                  <Trash2 className="h-4 w-4" />
                </button>
              )}
            </div>
          </div>

          {/* List */}
          <div className={clsx('overflow-y-auto', notifications.length === 0 ? 'py-8' : 'max-h-80')}>
            {notifications.length === 0 ? (
              <p className="text-center text-sm text-[--color-text-muted]">
                No notifications
              </p>
            ) : (
              <ul>
                {notifications.map((n) => (
                  <li key={n.id} className="border-b border-[--color-border-light] last:border-0">
                    <NotificationItem
                      notification={n}
                      onDismiss={dismissNotification}
                      onMarkRead={markAsRead}
                    />
                  </li>
                ))}
              </ul>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
