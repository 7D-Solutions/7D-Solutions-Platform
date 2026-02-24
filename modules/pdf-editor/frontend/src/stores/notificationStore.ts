// Notification and confirm dialog store.

import { create } from 'zustand';

export type NotificationType = 'success' | 'error' | 'info' | 'warning';

export interface Notification {
  id: string;
  type: NotificationType;
  message: string;
  duration?: number;
}

export interface ConfirmDialog {
  id: string;
  title: string;
  message: string;
  confirmLabel?: string;
  cancelLabel?: string;
  onConfirm: () => void;
  onCancel?: () => void;
  showDontAskAgain?: boolean;
  onDontAskAgain?: (dontAsk: boolean) => void;
}

let notifCounter = 0;
function nextId(): string {
  notifCounter += 1;
  return `notif-${Date.now()}-${notifCounter}`;
}

interface NotificationStore {
  notifications: Notification[];
  confirmDialog: ConfirmDialog | null;

  addNotification: (notification: Omit<Notification, 'id'>) => void;
  removeNotification: (id: string) => void;
  showConfirm: (dialog: Omit<ConfirmDialog, 'id'>) => void;
  hideConfirm: () => void;
}

export const useNotificationStore = create<NotificationStore>((set) => ({
  notifications: [],
  confirmDialog: null,

  addNotification: (notification) => {
    const id = nextId();
    const entry = { ...notification, id };

    set((state) => ({
      notifications: [...state.notifications, entry],
    }));

    const duration = notification.duration ?? 5000;
    setTimeout(() => {
      set((state) => ({
        notifications: state.notifications.filter((n) => n.id !== id),
      }));
    }, duration);
  },

  removeNotification: (id) =>
    set((state) => ({
      notifications: state.notifications.filter((n) => n.id !== id),
    })),

  showConfirm: (dialog) => {
    const id = nextId();
    set({ confirmDialog: { ...dialog, id } });
  },

  hideConfirm: () => set({ confirmDialog: null }),
}));
