import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

export default defineConfig({
  plugins: [react()],
  server: {
    proxy: {
      '^/node/':         { target: 'http://localhost:7000', changeOrigin: true },
      '^/cap/':          { target: 'http://localhost:7000', changeOrigin: true },
      '^/capabilities':  { target: 'http://localhost:7000', changeOrigin: true },
      '^/network/':      { target: 'http://localhost:7000', changeOrigin: true },
    }
  }
})
