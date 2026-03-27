import api from './client';

export interface CapabilityUi {
  label: string;
  icon?: string;
  entry: string;
  style: string;
  position?: 'left' | 'right';
}

export interface Capability {
  name: string;
  version: string;
  port: number;
  status: string | { Error: string };
  visibility: string;
  ui?: CapabilityUi;
  /** Short proxy route prefix (e.g. "feed"). Set at install time — authoritative. */
  route_name?: string;
}

export const getCapabilities = () =>
  api.get<{ capabilities: Capability[] }>('/capabilities').then(r => r.data.capabilities);
export const installCapability = (image: string) =>
  api.post('/capabilities/install', { image }).then(r => r.data);
export const stopCapability = (name: string) =>
  api.post(`/capabilities/${name}/stop`).then(r => r.data);
export const startCapability = (name: string) =>
  api.post(`/capabilities/${name}/start`).then(r => r.data);
export const uninstallCapability = (name: string) =>
  api.delete(`/capabilities/${name}`).then(r => r.data);
