// ============================================================
// Notification store — combines backend-persisted notifications
// (via TanStack Query) with local in-app notifications (ephemeral).
// The store manages local state; backend state is managed by
// useNotificationsQuery. The NotificationCenter merges both.
// ============================================================
'use client';
import { create } from 'zustand';
import { useShallow } from 'zustand/shallow';

export type NotificationSeverity = 'info' | 'success' | 'warning' | 'error';

export interface AppNotification {
  id: string;
  severity: NotificationSeverity;
  title: string;
  message?: string;
  timestamp: number;
  read: boolean;
  source: 'local' | 'backend';
}

interface NotificationStoreState {
  localNotifications: AppNotification[];
  localUnreadCount: number;
  lastSeenTimestamp: number;

  addNotification: (notification: Omit<AppNotification, 'id' | 'timestamp' | 'read' | 'source'>) => void;
  markAsRead: (id: string) => void;
  markAllAsRead: () => void;
  dismissNotification: (id: string) => void;
  clearAll: () => void;
  updateLastSeen: () => void;
}

const generateId = () => `notif_${Date.now()}_${Math.random().toString(36).substr(2, 9)}`;

export const useNotificationStore = create<NotificationStoreState>()((set) => ({
  localNotifications: [],
  localUnreadCount: 0,
  lastSeenTimestamp: 0,

  addNotification: (notification) => {
    const item: AppNotification = {
      ...notification,
      id: generateId(),
      timestamp: Date.now(),
      read: false,
      source: 'local',
    };
    set((state) => ({
      localNotifications: [item, ...state.localNotifications],
      localUnreadCount: state.localUnreadCount + 1,
    }));
  },

  markAsRead: (id) => {
    set((state) => {
      const updated = state.localNotifications.map((n) =>
        n.id === id && !n.read ? { ...n, read: true } : n
      );
      return {
        localNotifications: updated,
        localUnreadCount: updated.filter((n) => !n.read).length,
      };
    });
  },

  markAllAsRead: () => {
    set((state) => ({
      localNotifications: state.localNotifications.map((n) => ({ ...n, read: true })),
      localUnreadCount: 0,
    }));
  },

  dismissNotification: (id) => {
    set((state) => {
      const updated = state.localNotifications.filter((n) => n.id !== id);
      return {
        localNotifications: updated,
        localUnreadCount: updated.filter((n) => !n.read).length,
      };
    });
  },

  clearAll: () => set({ localNotifications: [], localUnreadCount: 0 }),

  updateLastSeen: () => set({ lastSeenTimestamp: Date.now() }),
}));

export const useLocalNotifications = () =>
  useNotificationStore((s) => s.localNotifications);
export const useLocalUnreadCount = () =>
  useNotificationStore((s) => s.localUnreadCount);

// Aliases consumed by NotificationCenter (merges local + backend in future)
export const useNotifications = useLocalNotifications;
export const useUnreadCount = useLocalUnreadCount;
export const useNotificationActions = () =>
  useNotificationStore(
    useShallow((s) => ({
      addNotification: s.addNotification,
      markAsRead: s.markAsRead,
      markAllAsRead: s.markAllAsRead,
      dismissNotification: s.dismissNotification,
      clearAll: s.clearAll,
      updateLastSeen: s.updateLastSeen,
    }))
  );
