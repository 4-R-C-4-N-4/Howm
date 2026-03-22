import axios from 'axios';

const api = axios.create({ baseURL: '/' });

// Attach Bearer token to all requests (some GET routes like /node/open-invite require auth)
api.interceptors.request.use((config) => {
  const token = getApiToken();
  if (token) {
    config.headers.Authorization = `Bearer ${token}`;
  }
  return config;
});

export default api;

/** Read the API token — injected into index.html by the daemon via <meta> tag. */
export function getApiToken(): string | null {
  // Prefer the daemon-injected meta tag (always fresh)
  const meta = document.querySelector('meta[name="howm-token"]');
  if (meta) {
    const token = meta.getAttribute('content');
    if (token) return token;
  }
  // Fall back to localStorage for dev mode (Vite proxy, token pasted manually)
  return localStorage.getItem('howm_api_token');
}

export function setApiToken(token: string) {
  localStorage.setItem('howm_api_token', token);
}

export function clearApiToken() {
  localStorage.removeItem('howm_api_token');
}
