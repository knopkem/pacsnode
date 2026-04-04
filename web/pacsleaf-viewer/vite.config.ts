import react from '@vitejs/plugin-react'
import { viteCommonjs } from '@originjs/vite-plugin-commonjs'
import { defineConfig } from 'vite'

export default defineConfig({
  base: '/viewer/',
  plugins: [react(), viteCommonjs()],
  optimizeDeps: {
    exclude: ['@cornerstonejs/dicom-image-loader'],
    include: ['dicom-parser'],
  },
  worker: {
    format: 'es',
  },
  build: {
    outDir: 'dist',
    sourcemap: true,
  },
})
