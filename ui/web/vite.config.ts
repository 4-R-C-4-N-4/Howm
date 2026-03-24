import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

// Proxy API requests to the Howm daemon.
// '/cap' is special: it's both an API prefix (iframe/XHR → daemon proxy)
// and a React route (/cap/:name → SPA page). We only proxy requests that
// look like API/asset fetches (not HTML page navigations) so the SPA
// fallback works for browser navigation.
const capProxy = {
  target: 'http://localhost:7000',
  changeOrigin: true,
  // Skip proxying when the browser is navigating (wants HTML) — let Vite
  // serve the SPA index.html so React Router can handle /cap/:name.
  bypass(req: any) {
    const accept = req.headers.accept || '';
    if (accept.includes('text/html')) return req.url;    // serve SPA
  },
};

const daemon = { target: 'http://localhost:7000', changeOrigin: true };

export default defineConfig({
  plugins: [react()],
  server: {
    proxy: {
      '/node':         daemon,
      '/cap':          capProxy,
      '/capabilities': daemon,
      '/network':      daemon,
      '/settings':     daemon,
      '/access':       daemon,
      '/theme.css':    daemon,
    }
  }
})
