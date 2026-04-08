import api from './client';

// ── Types ─────────────────────────────────────────────────────────────────────

export interface Profile {
  name: string;
  bio: string;
  avatar: string | null;
  homepage: string | null;
  has_avatar: boolean;
  has_homepage: boolean;
}

export interface PeerProfileCache {
  found: boolean;
  node_id?: string;
  name?: string;
  bio?: string;
  avatar_hash?: string | null;
  has_homepage?: boolean;
  updated_at?: number;
}

// ── API calls ─────────────────────────────────────────────────────────────────

export const getProfile = () =>
  api.get<Profile>('/profile').then(r => r.data);

export const updateProfile = (data: { name?: string; bio?: string }) =>
  api.put<{ name: string; bio: string }>('/profile', data).then(r => r.data);

export const uploadAvatar = (file: File) => {
  return api.put<{ avatar: string }>('/profile/avatar', file, {
    headers: { 'Content-Type': file.type },
  }).then(r => r.data);
};

export const setHomepage = (path: string | null) =>
  api.put<{ homepage: string | null }>('/profile/homepage', { path }).then(r => r.data);

export const getPeerProfile = (nodeId: string) =>
  api.get<Profile>(`/peer/${nodeId}/profile`).then(r => r.data);

export const getCachedProfile = (nodeId: string) =>
  api.get<PeerProfileCache>(`/profile/cache/${nodeId}`).then(r => r.data);
