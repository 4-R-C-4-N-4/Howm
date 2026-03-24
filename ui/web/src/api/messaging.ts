import api from './client';

// ── Types ──────────────────────────────────────────────

export interface Message {
  msg_id: string;
  conversation_id: string;
  direction: 'sent' | 'received';
  sender_peer_id: string;
  sent_at: number;
  body: string;
  delivery_status: 'pending' | 'delivered' | 'failed';
}

export interface LastMessage {
  msg_id: string;
  body_preview: string;
  sent_at: number;
  direction: 'sent' | 'received';
}

export interface ConversationSummary {
  conversation_id: string;
  peer_id: string;
  last_message: LastMessage | null;
  unread_count: number;
}

export interface ConversationPage {
  messages: Message[];
  next_cursor: number | null;
}

export interface SendResult {
  msg_id: string;
  status: 'delivered' | 'failed';
  sent_at: number;
}

// ── API calls ──────────────────────────────────────────

export const sendMessage = (to: string, body: string) =>
  api.post<SendResult>('/cap/messaging/send', { to, body }).then(r => r.data);

export const getConversations = () =>
  api.get<ConversationSummary[]>('/cap/messaging/conversations').then(r => r.data);

export const getConversation = (peerId: string, cursor?: number, limit = 50) =>
  api.get<ConversationPage>(`/cap/messaging/conversations/${peerId}`, {
    params: { cursor, limit },
  }).then(r => r.data);

export const markRead = (peerId: string) =>
  api.post(`/cap/messaging/conversations/${peerId}/read`);

export const deleteMessage = (peerId: string, msgId: string) =>
  api.delete(`/cap/messaging/conversations/${peerId}/messages/${msgId}`);
