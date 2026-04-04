import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { BrowserRouter } from 'react-router-dom'

import App from './App'
import './index.css'
import { getRuntimeConfig } from './lib/runtime-config'

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      staleTime: 30_000,
      refetchOnWindowFocus: false,
      retry: 1,
    },
  },
})

const rootElement = document.getElementById('root')

if (!rootElement) {
  throw new Error('Unable to find root element for pacsleaf viewer.')
}

const runtimeConfig = getRuntimeConfig()

createRoot(rootElement).render(
  <StrictMode>
    <QueryClientProvider client={queryClient}>
      <BrowserRouter basename={runtimeConfig.routerBasename}>
        <App />
      </BrowserRouter>
    </QueryClientProvider>
  </StrictMode>,
)
