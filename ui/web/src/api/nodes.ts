import api from './client';

export interface NodeInfo {
  node_id: string;
  name: string;
  created: number;
  wg_pubkey: string | null;
  wg_address: string | null;
  wg_endpoint: string | null;
}

export type TrustLevel = 'friend' | 'public' | 'restricted';

export interface Peer {
  node_id: string;
  name: string;
  wg_pubkey: string;
  wg_address: string;
  wg_endpoint: string;
  port: number;
  last_seen: number;
  trust: TrustLevel;
}

export interface OpenInviteStatus {
  enabled: boolean;
  link?: string;
  label?: string;
  max_peers?: number;
  current_peer_count?: number;
  created_at?: number;
  expires_at?: number | null;
}

export interface WgStatus {
  status: string;
  public_key: string | null;
  address: string | null;
  endpoint: string | null;
  listen_port: number | null;
  active_tunnels: number | null;
  peers: WgPeerStatus[] | null;
}

export interface WgPeerStatus {
  public_key: string;
  endpoint: string | null;
  allowed_ips: string | null;
  latest_handshake: number | null;
  transfer_rx: number | null;
  transfer_tx: number | null;
}

export const getNodeInfo = () => api.get<NodeInfo>('/node/info').then(r => r.data);
export const getPeers = () => api.get<{ peers: Peer[] }>('/node/peers').then(r => r.data.peers);
export const removePeer = (node_id: string) =>
  api.delete(`/node/peers/${node_id}`).then(r => r.data);
export const getWgStatus = () => api.get<WgStatus>('/node/wireguard').then(r => r.data);
export const generateInvite = (endpoint?: string) =>
  api.post<{ invite_code: string }>('/node/invite', endpoint ? { endpoint } : {}).then(r => r.data);
export const redeemInvite = (invite_code: string) =>
  api.post('/node/redeem-invite', { invite_code }).then(r => r.data);

export const getOpenInvite = () =>
  api.get<OpenInviteStatus>('/node/open-invite').then(r => r.data);
export const createOpenInvite = (label?: string, max_peers?: number) =>
  api.post<OpenInviteStatus>('/node/open-invite', { label, max_peers }).then(r => r.data);
export const revokeOpenInvite = () =>
  api.delete('/node/open-invite').then(r => r.data);
export const redeemOpenInvite = (invite_link: string) =>
  api.post('/node/redeem-open-invite', { invite_link }).then(r => r.data);
export const updatePeerTrust = (node_id: string, trust: TrustLevel) =>
  api.patch(`/node/peers/${node_id}/trust`, { trust }).then(r => r.data);
