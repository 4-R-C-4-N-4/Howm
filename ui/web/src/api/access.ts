import api from './client';

// ── Types ──────────────────────────────────────────────

export interface AccessGroup {
  group_id: string;
  name: string;
  built_in: boolean;
  description: string | null;
  capabilities: CapabilityRule[];
  created_at: number;
}

export interface CapabilityRule {
  capability_name: string;
  allow: boolean;
  rate_limit: number | null;
  ttl: number | null;
}

export interface PeerPermissions {
  peer_id: string;
  permissions: Record<string, {
    allowed: boolean;
    rate_limit?: number | null;
    ttl?: number | null;
  }>;
}

// ── Well-known UUIDs ───────────────────────────────────

export const GROUP_DEFAULT  = '00000000-0000-0000-0000-000000000001';
export const GROUP_FRIENDS  = '00000000-0000-0000-0000-000000000002';
export const GROUP_TRUSTED  = '00000000-0000-0000-0000-000000000003';

// ── Well-known capability sets per built-in tier ───────

export const TIER_CAPABILITIES: Record<string, string[]> = {
  [GROUP_DEFAULT]: [
    'core.session.heartbeat.1',
    'core.session.attest.1',
    'core.session.latency.1',
    'core.network.endpoint.1',
    'core.session.timesync.1',
  ],
  [GROUP_FRIENDS]: [
    'core.session.heartbeat.1',
    'core.session.attest.1',
    'core.session.latency.1',
    'core.network.endpoint.1',
    'core.session.timesync.1',
    'howm.feed.1',
    'howm.social.messaging.1',
    'howm.social.files.1',
    'howm.world.room.1',
    'core.network.peerexchange.1',
  ],
  [GROUP_TRUSTED]: [
    'core.session.heartbeat.1',
    'core.session.attest.1',
    'core.session.latency.1',
    'core.network.endpoint.1',
    'core.session.timesync.1',
    'howm.feed.1',
    'howm.social.messaging.1',
    'howm.social.files.1',
    'howm.world.room.1',
    'core.network.peerexchange.1',
    'core.network.relay.1',
  ],
};

// ── Group API ──────────────────────────────────────────

export const getAccessGroups = () =>
  api.get<AccessGroup[]>('/access/groups').then(r => r.data);

export const createAccessGroup = (name: string, description?: string, capabilities?: CapabilityRule[]) =>
  api.post<AccessGroup>('/access/groups', { name, description, capabilities }).then(r => r.data);

export const getAccessGroup = (groupId: string) =>
  api.get<AccessGroup>(`/access/groups/${groupId}`).then(r => r.data);

export const updateAccessGroup = (groupId: string, updates: {
  name?: string;
  description?: string | null;
  capabilities?: CapabilityRule[];
}) =>
  api.put<AccessGroup>(`/access/groups/${groupId}`, updates).then(r => r.data);

export const deleteAccessGroup = (groupId: string) =>
  api.delete(`/access/groups/${groupId}`).then(r => r.data);

// ── Peer Group Membership API ──────────────────────────

export const getPeerGroups = (peerId: string) =>
  api.get<AccessGroup[]>(`/access/peers/${peerId}/groups`).then(r => r.data);

export const assignPeerToGroup = (peerId: string, groupId: string) =>
  api.post(`/access/peers/${peerId}/groups`, { group_id: groupId }).then(r => r.data);

export const removePeerFromGroup = (peerId: string, groupId: string) =>
  api.delete(`/access/peers/${peerId}/groups/${groupId}`).then(r => r.data);

// ── Permissions API ────────────────────────────────────

export const getPeerPermissions = (peerId: string) =>
  api.get<PeerPermissions>(`/access/peers/${peerId}/permissions`).then(r => r.data);

// ── Deny API ───────────────────────────────────────────

export const denyPeer = (peerId: string) =>
  api.post(`/access/peers/${peerId}/deny`).then(r => r.data);

// ── Convenience: Move peer to a built-in tier ──────────

export async function movePeerToTier(peerId: string, targetGroupId: string): Promise<void> {
  const currentGroups = await getPeerGroups(peerId);
  const builtInGroups = currentGroups.filter(g => g.built_in);

  // Remove from all current built-in groups
  await Promise.all(
    builtInGroups
      .filter(g => g.group_id !== targetGroupId)
      .map(g => removePeerFromGroup(peerId, g.group_id))
  );

  // Assign to target if not already in it
  if (!builtInGroups.some(g => g.group_id === targetGroupId)) {
    await assignPeerToGroup(peerId, targetGroupId);
  }
}
