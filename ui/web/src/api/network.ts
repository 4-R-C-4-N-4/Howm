import api from './client';
import type { WgStatus } from './nodes';

// ── Types ────────────────────────────────────────────────────────────────────

export type NatType = 'open' | 'cone' | 'symmetric' | 'unknown';
export type Reachability = 'direct' | 'punchable' | 'relay-only' | 'unknown';
export type ExchangeStatus = 'waiting' | 'completed' | 'expired';

export interface NatProfile {
  detected: boolean;
  nat_type: NatType;
  external_ipv4: string | null;
  external_port: number | null;
  observed_stride: number;
  detected_at: number;
}

export interface IPv6Status {
  available: boolean;
  global_addresses: string[];
  preferred: boolean;
}

export interface RelayConfig {
  allow_relay: boolean;
  relay_capable_peers: number;
}

export interface NetworkStatus {
  wireguard: WgStatus;
  nat: NatProfile | null;
  ipv6: IPv6Status;
  reachability: Reachability;
  relay: RelayConfig;
  peer_count: number;
}

export interface PendingExchange {
  id: string;
  created_at: number;
  expires_at: number;
  status: ExchangeStatus;
  time_remaining_secs: number;
}

// ── API calls ────────────────────────────────────────────────────────────────

export const getNetworkStatus = () =>
  api.get<NetworkStatus>('/network/status').then(r => r.data);

export const detectNetwork = () =>
  api.post<NatProfile>('/network/detect').then(r => r.data);

export const getNatProfile = () =>
  api.get<NatProfile>('/network/nat-profile').then(r => r.data);

export const updateRelayConfig = (allow_relay: boolean) =>
  api.put<RelayConfig>('/network/relay', { allow_relay }).then(r => r.data);

export const getPendingExchanges = () =>
  api.get<{ pending: PendingExchange[] }>('/network/pending').then(r => r.data.pending);

export const redeemAccept = (accept_token: string) =>
  api.post('/node/accept', { accept_token }).then(r => r.data);
