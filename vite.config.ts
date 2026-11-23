import { readFileSync } from 'node:fs'
import { fileURLToPath, URL } from 'node:url'
import tailwindcss from '@tailwindcss/vite'
import vue from '@vitejs/plugin-vue'
import { defineConfig } from 'vitest/config'
import { rainPipeProxy } from './vite/pipe-proxy.ts'

// The one source of the version text in Settings > About.
const pkg = JSON.parse(readFileSync(new URL('./package.json', import.meta.url), 'utf8')) as { version: string }

// Tauri expects a fixed port and fails if it is taken (strictPort).
const TAURI_DEV_PORT = 1420

export default defineConfig({
  define: { __APP_VERSION__: JSON.stringify(pkg.version) },
  // rainPipeProxy lets a plain browser tab reach the backend (development only).
  plugins: [vue(), tailwindcss(), rainPipeProxy()],
  resolve: {
    alias: {
      '@': fileURLToPath(new URL('./src', import.meta.url)),
    },
  },
  // Keep Rust compiler errors visible in the terminal when running under Tauri.
  clearScreen: false,
  server: {
    port: TAURI_DEV_PORT,
    strictPort: true,
    watch: {
      // Rust sources and build output change while the backend is rebuilt; the page does not care.
      ignored: ['**/src-tauri/**', '**/crates/**', '**/target/**', '**/.rain-*/**'],
    },
  },
  test: {
    // Unit tests sit next to the code they test.
    include: ['src/**/*.test.ts', 'vite/**/*.test.ts'],
  },
})
