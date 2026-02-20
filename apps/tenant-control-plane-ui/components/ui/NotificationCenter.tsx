'use client';
// ============================================================
// NotificationCenter — bell icon + badge + notification dropdown
// Merges backend-persisted notifications (via TanStack Query)
// with local in-app notifications (ephemeral Zustand store).
// ============================================================
import { useState, useRef, useEffect, useMemo } from 'react';
import { clsx } from 'clsx';
import { Bell, CheckCheck, Trash2 } from 'lucide-react';
import {
  useLocalNotifications,
  useLocalUnreadCount,
  useNotificationActions,
} from '@/infrastructure/state/notificationStore';
import { useNotificationsQuery } from '@/infrastructure/hooks/useNotificationsQuery';
import { NotificationItem } from './NotificationItem';
import type { AppNotification } from '@/infrastructure/state/notificationStore';

export function NotificationCenter() {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  // Backend-persisted notifications
  const {
    notifications: backendNotifications,
    unreadCount: backendUnreadCount,
    markAsRead: markBackendRead,
    markAllAsRead: markAllBackendRead,
  } = useNotificationsQuery();

  // Local ephemeral notifications
  const localNotifications = useLocalNotifications();
  const localUnreadCount = useLocalUnreadCount();
  const {
    markAsRead: markLocalRead,
    markAllAsRead: markAllLocalRead,
    dismissNotification: dismissLocal,
    clearAll: clearLocalAll,
    updateLastSeen,
  } = useNotificationActions();

  // Merge backend + local, sorted newest first
  const allNotifications = useMemo<AppNotification[]>(() => {
    const backendMapped: AppNotification[] = backendNotifications.map((n) => ({
      id: n.id,
      severity: n.severity as AppNotification['severity'],
      title: n.title,
      message: n.message,
      timestamp: new Date(n.timestamp).getTime(),
      read: n.read,
      source: 'backend' as const,
    }));
    return [...localNotifications, ...backendMapped].sort(
      (a, b) => b.timestamp - a.timestamp,
    );
  }, [backendNotifications, localNotifications]);

  const totalUnread = localUnreadCount + backendUnreadCount;

  useEffect(() => {
    const handleClickOutside = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    document.addEventListener('mousedown', handleClickOutside);
    return () => document.removeEventListener('mousedown', handleClickOutside);
  }, []);

  const handleMarkRead = (id: string, source: 'local' | 'backend') => {
    if (source === 'local') {
      markLocalRead(id);
    } else {
      markBackendRead([id]);
    }
  };

  const handleMarkAllRead = () => {
    markAllLocalRead();
    markAllBackendRead();
  };

  const handleDismiss = (id: string, source: 'local' | 'backend') => {
    if (source === 'local') {
      dismissLocal(id);
    }
    // Backend notifications are dismissed via mark-read only
  };

  const handleClearAll = () => {
    clearLocalAll();
    markAllBackendRead();
  };

  const handleOpen = () => {
    setOpen(!open);
    if (!open) {
      updateLastSeen();
    }
  };

  return (
    <div ref={ref} className="relative">
      <button
        onClick={handleOpen}
        className="relative rounded-[--radius-default] p-2 text-[--color-text-secondary] hover:bg-[--color-bg-tertiary] transition-[--transition-fast]"
        aria-label={`Notifications${totalUnread > 0 ? `, ${totalUnread} unread` : ''}`}
        data-testid="notification-bell"
      >
        <Bell className="h-5 w-5" />
        {totalUnread > 0 && (
          <span
            className="absolute -right-0.5 -top-0.5 flex h-4 w-4 items-center justify-center rounded-full bg-[--color-danger] text-[10px] font-bold text-white"
            data-testid="notification-badge"
          >
            {totalUnread > 99 ? '99+' : totalUnread}
          </span>
        )}
      </button>

      {open && (
        <div
          className="absolute right-0 mt-1 w-80 rounded-[--radius-lg] border border-[--color-border-light] bg-[--color-bg-primary] shadow-[--shadow-xl]"
          style={{ zIndex: 'var(--z-dropdown)' }}
          data-testid="notification-panel"
        >
          {/* Header */}
          <div className="flex items-center justify-between border-b border-[--color-border-light] px-4 py-3">
            <h3 className="text-sm font-semibold text-[--color-text-primary]">
              Notifications
              {totalUnread > 0 && (
                <span className="ml-2 rounded-full bg-[--color-danger] px-1.5 py-0.5 text-[10px] text-white">
                  {totalUnread}
                </span>
              )}
            </h3>
            <div className="flex items-center gap-1">
              {totalUnread > 0 && (
                <button
                  onClick={handleMarkAllRead}
                  title="Mark all as read"
                  className="rounded p-1 text-[--color-text-secondary] hover:bg-[--color-bg-tertiary]"
                  data-testid="mark-all-read"
                >
                  <CheckCheck className="h-4 w-4" />
                </button>
              )}
              {allNotifications.length > 0 && (
                <button
                  onClick={handleClearAll}
                  title="Clear all"
                  className="rounded p-1 text-[--color-text-secondary] hover:bg-[--color-bg-tertiary]"
                  data-testid="clear-all-notifications"
                >
                  <Trash2 className="h-4 w-4" />
                </button>
              )}
            </div>
          </div>

          {/* List */}
          <div className={clsx('overflow-y-auto', allNotifications.length === 0 ? 'py-8' : 'max-h-80')}>
            {allNotifications.length === 0 ? (
              <p
                className="text-center text-sm text-[--color-text-muted]"
                data-testid="notification-empty"
              >
                No notifications
              </p>
            ) : (
              <ul>
                {allNotifications.map((n) => (
                  <li key={n.id} className="border-b border-[--color-border-light] last:border-0">
                    <NotificationItem
                      notification={n}
                      onDismiss={(id) => handleDismiss(id, n.source)}
                      onMarkRead={(id) => handleMarkRead(id, n.source)}
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
