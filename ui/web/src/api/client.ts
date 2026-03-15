import axios from 'axios';

const api = axios.create({ baseURL: '/' });

// Attach Bearer token to mutating requests (POST, PUT, DELETE)
api.interceptors.request.use((config) => {
  const token = localStorage.getItem('howm_api_token');
  if (token && config.method && ['post', 'put', 'delete', 'patch'].includes(config.method)) {
    config.headers.Authorization = `Bearer ${token}`;
  }
  return config;
});

export default api;

export function getApiToken(): string | null {
  return localStorage.getItem('howm_api_token');
}

export function setApiToken(token: string) {
  localStorage.setItem('howm_api_token', token);
}

export function clearApiToken() {
  localStorage.removeItem('howm_api_token');
}
