import { defineConfig } from 'vite'
import { devtools } from '@tanstack/devtools-vite'
import { tanstackRouter } from '@tanstack/router-plugin/vite'
import viteReact from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'

const config = defineConfig({
  base: '/',
  resolve: {
    tsconfigPaths: true,
  },
  plugins: [
    devtools(),
    tanstackRouter({
      routeFileIgnorePattern: '.*/components/.*',
    }),
    tailwindcss(),
    viteReact(),
  ],
  build: {
    outDir: 'dist',
    emptyOutDir: true,
  },
})

export default config
