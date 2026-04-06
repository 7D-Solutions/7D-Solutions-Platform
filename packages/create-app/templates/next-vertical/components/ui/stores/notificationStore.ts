type Subscriber = () => void;

export type NotificationVariant = "default" | "success" | "warning" | "danger" | "info";

export interface Notification {
  id: string;
  message: string;
  variant?: NotificationVariant;
  duration?: number;
  action?: { label: string; onClick: () => void };
}

let notifications: Notification[] = [];
const subscribers = new Set<Subscriber>();
let nextId = 1;

function notify(): void {
  subscribers.forEach((fn) => fn());
}

export const notificationStore = {
  subscribe(fn: Subscriber): () => void {
    subscribers.add(fn);
    return () => subscribers.delete(fn);
  },

  getSnapshot(): readonly Notification[] {
    return notifications;
  },

  add(notification: Omit<Notification, "id">): string {
    const id = `notification-${nextId++}`;
    notifications = [...notifications, { ...notification, id }];
    notify();
    return id;
  },

  dismiss(id: string): void {
    notifications = notifications.filter((n) => n.id !== id);
    notify();
  },

  clear(): void {
    notifications = [];
    notify();
  },
};
