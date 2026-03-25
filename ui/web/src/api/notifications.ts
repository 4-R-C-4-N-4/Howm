import api from './client';

// ── Types ──────────────────────────────────────────────

export interface BadgesResponse {
  badges: Record<string, number>;
}

export interface Notification {
  id: string;
  capability: string;
  level: 'info' | 'success' | 'warning' | 'error';
  title: string;
  message: string;
  action?: string;
  created_at: number;
}

export interface PollResponse {
  notifications: Notification[];
  timestamp: number;
}

// ── API calls ──────────────────────────────────────────

export const getBadges = () =>
  api.get<BadgesResponse>('/notifications/badges').then(r => r.data);

export const pollNotifications = (since: number) =>
  api.get<PollResponse>('/notifications/poll', {
    params: { since },
  }).then(r => r.data);
