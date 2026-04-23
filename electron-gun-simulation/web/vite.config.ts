import { defineConfig } from 'vite';

export default defineConfig({
  optimizeDeps: {
    exclude: ['wasm-api'],
  },
});
