// ============================================================
// In-app notification store — in-memory only (not persisted)
// Used by NotificationCenter component.
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
}

interface NotificationStoreState {
  notifications: AppNotification[];
  unreadCount: number;

  addNotification: (notification: Omit<AppNotification, 'id' | 'timestamp' | 'read'>) => void;
  markAsRead: (id: string) => void;
  markAllAsRead: () => void;
  dismissNotification: (id: string) => void;
  clearAll: () => void;
}

const generateId = () => `notif_${Date.now()}_${Math.random().toString(36).substr(2, 9)}`;

export const useNotificationStore = create<NotificationStoreState>()((set) => ({
  notifications: [],
  unreadCount: 0,

  addNotification: (notification) => {
    const item: AppNotification = {
      ...notification,
      id: generateId(),
      timestamp: Date.now(),
      read: false,
    };
    set((state) => ({
      notifications: [item, ...state.notifications],
      unreadCount: state.unreadCount + 1,
    }));
  },

  markAsRead: (id) => {
    set((state) => {
      const updated = state.notifications.map((n) =>
        n.id === id && !n.read ? { ...n, read: true } : n
      );
      return {
        notifications: updated,
        unreadCount: updated.filter((n) => !n.read).length,
      };
    });
  },

  markAllAsRead: () => {
    set((state) => ({
      notifications: state.notifications.map((n) => ({ ...n, read: true })),
      unreadCount: 0,
    }));
  },

  dismissNotification: (id) => {
    set((state) => {
      const updated = state.notifications.filter((n) => n.id !== id);
      return {
        notifications: updated,
        unreadCount: updated.filter((n) => !n.read).length,
      };
    });
  },

  clearAll: () => set({ notifications: [], unreadCount: 0 }),
}));

export const useNotifications = () => useNotificationStore((s) => s.notifications);
export const useUnreadCount = () => useNotificationStore((s) => s.unreadCount);
export const useNotificationActions = () =>
  useNotificationStore(
    useShallow((s) => ({
      addNotification: s.addNotification,
      markAsRead: s.markAsRead,
      markAllAsRead: s.markAllAsRead,
      dismissNotification: s.dismissNotification,
      clearAll: s.clearAll,
    }))
  );
