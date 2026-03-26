import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'

// Proxy API and capability requests to the Howm daemon.
// SPA routes live under /app, /dashboard, /peers, etc. — no conflict with
// these proxy prefixes. The /cap prefix is exclusively for the daemon's
// capability proxy (iframe content + API calls from capability UIs).
const daemon = { target: 'http://localhost:7000', changeOrigin: true };

export default defineConfig({
  plugins: [react(), tailwindcss()],
  server: {
    proxy: {
      '/node':         daemon,
      '/cap':          daemon,
      '/capabilities': daemon,
      '/network':      daemon,
      '/settings':     daemon,
      '/access':       daemon,
      '/theme.css':    daemon,
    }
  }
})
