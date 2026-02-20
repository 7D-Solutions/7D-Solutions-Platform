// ============================================================
// useNotificationsQuery — TanStack Query hook for notification center
// Fetches from BFF /api/notifications with polling.
// Mark-read calls POST /api/notifications/mark-read and invalidates.
// ============================================================
'use client';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { NOTIFICATION_POLL_MS } from '@/lib/constants';
import type { NotificationListResponse, MarkReadRequest } from '@/lib/api/types';

const QUERY_KEY = ['notifications'] as const;

async function fetchNotifications(): Promise<NotificationListResponse> {
  const res = await fetch('/api/notifications');
  if (!res.ok) {
    return { notifications: [], unread_count: 0 };
  }
  return res.json();
}

async function postMarkRead(body: MarkReadRequest): Promise<void> {
  await fetch('/api/notifications/mark-read', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
  });
}

export function useNotificationsQuery() {
  const queryClient = useQueryClient();

  const { data, isLoading } = useQuery<NotificationListResponse>({
    queryKey: [...QUERY_KEY],
    queryFn: fetchNotifications,
    refetchInterval: NOTIFICATION_POLL_MS,
    staleTime: NOTIFICATION_POLL_MS / 2,
    retry: false,
  });

  const markReadMutation = useMutation({
    mutationFn: postMarkRead,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: [...QUERY_KEY] });
    },
  });

  return {
    notifications: data?.notifications ?? [],
    unreadCount: data?.unread_count ?? 0,
    isLoading,
    markAsRead: (ids: string[]) => markReadMutation.mutate({ ids }),
    markAllAsRead: () => markReadMutation.mutate({ all: true }),
  };
}
