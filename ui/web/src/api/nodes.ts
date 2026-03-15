import api from './client';

export interface NodeInfo {
  node_id: string;
  name: string;
  created: number;
  tailnet_ip: string | null;
  tailnet_name: string | null;
}

export interface Peer {
  node_id: string;
  address: string;
  name: string;
  port: number;
  last_seen: number;
}

export interface TailnetStatus {
  tailnet_ip: string | null;
  tailnet_name: string | null;
  coordination_url: string | null;
  status: string;
}

export interface AuthKey {
  prefix: string;
}

export const getNodeInfo = () => api.get<NodeInfo>('/node/info').then(r => r.data);
export const getPeers = () => api.get<{ peers: Peer[] }>('/node/peers').then(r => r.data.peers);
export const addPeer = (address: string, port: number, auth_key?: string) =>
  api.post('/node/peers', { address, port, auth_key }).then(r => r.data);
export const removePeer = (node_id: string) =>
  api.delete(`/node/peers/${node_id}`).then(r => r.data);
export const getTailnet = () => api.get<TailnetStatus>('/node/tailnet').then(r => r.data);
export const generateInvite = (address?: string) =>
  api.post<{ invite_code: string }>('/node/invite', address ? { address } : {}).then(r => r.data);
export const redeemInvite = (invite_code: string) =>
  api.post('/node/redeem-invite', { invite_code }).then(r => r.data);
export const getAuthKeys = () =>
  api.get<{ keys: AuthKey[] }>('/node/auth-keys').then(r => r.data.keys);
export const addAuthKey = (key: string) =>
  api.post('/node/auth-keys', { key }).then(r => r.data);
export const removeAuthKey = (prefix: string) =>
  api.delete(`/node/auth-keys/${prefix}`).then(r => r.data);
