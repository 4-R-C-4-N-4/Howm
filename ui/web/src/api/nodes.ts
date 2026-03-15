import api from './client';

export interface NodeInfo {
  node_id: string;
  name: string;
  created: number;
  wg_pubkey: string | null;
  wg_address: string | null;
  wg_endpoint: string | null;
}

export interface Peer {
  node_id: string;
  name: string;
  wg_pubkey: string;
  wg_address: string;
  wg_endpoint: string;
  port: number;
  last_seen: number;
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
