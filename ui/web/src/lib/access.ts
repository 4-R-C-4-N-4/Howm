export const GROUP_DEFAULT  = '00000000-0000-0000-0000-000000000001';
export const GROUP_FRIENDS  = '00000000-0000-0000-0000-000000000002';
export const GROUP_TRUSTED  = '00000000-0000-0000-0000-000000000003';

export const BUILT_IN_TIERS = [
  { id: GROUP_DEFAULT, label: 'Default', color: '#9ca3af', bg: 'rgba(156,163,175,0.12)', order: 0 },
  { id: GROUP_FRIENDS, label: 'Friends', color: '#60a5fa', bg: 'rgba(96,165,250,0.12)', order: 1 },
  { id: GROUP_TRUSTED, label: 'Trusted', color: '#fbbf24', bg: 'rgba(251,191,36,0.12)', order: 2 },
] as const;

export interface TierBadge {
  label: string;
  color: string;
  bg: string;
}

export function effectiveTier(groups: { group_id: string; built_in: boolean }[]): TierBadge {
  const builtInIds = new Set(groups.filter(g => g.built_in).map(g => g.group_id));
  if (builtInIds.has(GROUP_TRUSTED)) return { label: 'Trusted', color: '#fbbf24', bg: 'rgba(251,191,36,0.12)' };
  if (builtInIds.has(GROUP_FRIENDS)) return { label: 'Friends', color: '#60a5fa', bg: 'rgba(96,165,250,0.12)' };
  if (builtInIds.has(GROUP_DEFAULT)) return { label: 'Default', color: '#9ca3af', bg: 'rgba(156,163,175,0.12)' };
  if (groups.length > 0)            return { label: 'Custom',  color: '#c084fc', bg: 'rgba(192,132,252,0.12)' };
  return { label: 'Denied', color: '#f87171', bg: 'rgba(248,113,113,0.12)' };
}

export function peerIdToHex(pubkey: string): string {
  const bytes = atob(pubkey);
  return Array.from(bytes, b => b.charCodeAt(0).toString(16).padStart(2, '0')).join('');
}

export function hexToPubkey(hex: string): string {
  const bytes = hex.match(/.{2}/g)!.map(b => parseInt(b, 16));
  return btoa(String.fromCharCode(...bytes));
}

export function formatLastSeen(ts: number, now: number): string {
  if (!ts) return 'never';
  const delta = Math.floor(now / 1000 - ts);
  if (delta < 60) return 'just now';
  if (delta < 3600) return `${Math.floor(delta / 60)}m ago`;
  if (delta < 86400) return `${Math.floor(delta / 3600)}h ago`;
  return `${Math.floor(delta / 86400)}d ago`;
}

export function isOnline(lastSeen: number, now: number): boolean {
  if (!lastSeen) return false;
  return Math.floor(now / 1000 - lastSeen) < 90;
}

// All known capabilities for group creation form
export const ALL_CAPABILITIES = [
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
];

export const CORE_CAPABILITIES = [
  'core.session.heartbeat.1',
  'core.session.attest.1',
  'core.session.latency.1',
  'core.network.endpoint.1',
  'core.session.timesync.1',
];
