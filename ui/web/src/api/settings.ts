import api from './client';

export interface NodeSettings {
  node_id: string;
  name: string;
  wg_address: string;
  listen_port: number;
  data_dir: string;
}

export interface IdentitySettings {
  public_key: string;
  display_name: string;
}

export interface P2pcdConfig {
  heartbeat_interval_secs?: number;
  heartbeat_timeout_secs?: number;
  discovery_port?: number;
  [key: string]: unknown;
}

export const getNodeSettings = () =>
  api.get<NodeSettings>('/settings/node').then(r => r.data);

export const getIdentity = () =>
  api.get<IdentitySettings>('/settings/identity').then(r => r.data);

export const getP2pcdConfig = () =>
  api.get<P2pcdConfig>('/settings/p2pcd').then(r => r.data);

export const updateP2pcdConfig = (patch: Partial<P2pcdConfig>) =>
  api.put<P2pcdConfig>('/settings/p2pcd', patch).then(r => r.data);
