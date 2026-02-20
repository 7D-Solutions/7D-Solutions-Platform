'use client';
// ============================================================
// NotificationItem — single notification row in the dropdown
// ============================================================
import { clsx } from 'clsx';
import { CheckCircle, AlertTriangle, XCircle, Info, X } from 'lucide-react';
import type { AppNotification } from '@/infrastructure/state/notificationStore';
import { formatDateTime } from '@/infrastructure/utils/formatters';

const severityConfig = {
  info:    { Icon: Info,          colorClass: 'text-[--color-info]' },
  success: { Icon: CheckCircle,   colorClass: 'text-[--color-success]' },
  warning: { Icon: AlertTriangle, colorClass: 'text-[--color-warning]' },
  error:   { Icon: XCircle,       colorClass: 'text-[--color-danger]' },
};

interface NotificationItemProps {
  notification: AppNotification;
  onDismiss: (id: string) => void;
  onMarkRead: (id: string) => void;
}

export function NotificationItem({ notification, onDismiss, onMarkRead }: NotificationItemProps) {
  const { Icon, colorClass } = severityConfig[notification.severity];

  return (
    <div
      onClick={() => !notification.read && onMarkRead(notification.id)}
      className={clsx(
        'flex items-start gap-3 px-4 py-3 hover:bg-[--color-bg-secondary] transition-[--transition-fast] cursor-pointer',
        !notification.read && 'bg-blue-50'
      )}
    >
      <Icon className={clsx('mt-0.5 h-4 w-4 flex-shrink-0', colorClass)} />

      <div className="flex-1 min-w-0">
        <p className={clsx('text-sm', !notification.read && 'font-medium')}>{notification.title}</p>
        {notification.message && (
          <p className="mt-0.5 text-xs text-[--color-text-secondary] line-clamp-2">{notification.message}</p>
        )}
        <p className="mt-1 text-xs text-[--color-text-muted]">
          {formatDateTime(new Date(notification.timestamp))}
        </p>
      </div>

      <button
        onClick={(e) => { e.stopPropagation(); onDismiss(notification.id); }}
        className="flex-shrink-0 rounded p-0.5 hover:bg-[--color-bg-tertiary]"
        aria-label="Dismiss notification"
      >
        <X className="h-3.5 w-3.5 text-[--color-text-muted]" />
      </button>
    </div>
  );
}
