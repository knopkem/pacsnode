import { create } from 'zustand'
import { createJSONStorage, persist } from 'zustand/middleware'

import { getRuntimeConfig } from '../lib/runtime-config'
import type { RenderingMode, StreamingQuality } from '../lib/runtime-config'
import type { ClientEngine } from '../lib/engines/types'

const runtimeConfig = getRuntimeConfig()

function normalizeLimit(value: number): number {
  if (!Number.isFinite(value)) {
    return 25
  }

  return Math.min(250, Math.max(10, Math.round(value)))
}

/**
 * Resolve the effective streamer URL from the server-injected runtime config.
 * If the admin configured `localhost` or `127.0.0.1`, replace with the
 * hostname the browser used to reach pacsnode so streaming works for remote
 * clients too.
 */
export function resolveStreamerUrl(): string {
  const raw = runtimeConfig.streaming.defaultUrl.trim()
  if (!raw) return ''

  try {
    const parsed = new URL(raw)
    if (parsed.hostname === 'localhost' || parsed.hostname === '127.0.0.1') {
      parsed.hostname = window.location.hostname
    }
    return parsed.toString().replace(/\/+$/, '')
  } catch {
    return raw
  }
}

export const viewerPreferenceDefaults = {
  defaultStudyLimit: 25,
  autoSelectFirstSeries: runtimeConfig.viewer.autoSelectFirstSeries,
  preferredRenderingMode: runtimeConfig.rendering.defaultMode as RenderingMode,
  streamingQuality: runtimeConfig.streaming.defaultQuality as StreamingQuality,
  viewportLayout: 'single' as const,
  clientEngine: 'auto' as ClientEngine,
}

interface ViewerPreferencesState {
  defaultStudyLimit: number
  autoSelectFirstSeries: boolean
  preferredRenderingMode: RenderingMode
  streamingQuality: StreamingQuality
  viewportLayout: 'single' | 'quad'
  clientEngine: ClientEngine
  setDefaultStudyLimit: (value: number) => void
  setAutoSelectFirstSeries: (value: boolean) => void
  setPreferredRenderingMode: (value: RenderingMode) => void
  setStreamingQuality: (value: StreamingQuality) => void
  setViewportLayout: (value: 'single' | 'quad') => void
  setClientEngine: (value: ClientEngine) => void
  reset: () => void
}

export const useViewerPreferencesStore = create<ViewerPreferencesState>()(
  persist(
    (set) => ({
      ...viewerPreferenceDefaults,
        setDefaultStudyLimit: (value) =>
          set({
            defaultStudyLimit: normalizeLimit(value),
          }),
        setAutoSelectFirstSeries: (value) => set({ autoSelectFirstSeries: value }),
        setPreferredRenderingMode: (value) => set({ preferredRenderingMode: value }),
        setStreamingQuality: (value) => set({ streamingQuality: value }),
        setViewportLayout: (value) => set({ viewportLayout: value }),
        setClientEngine: (value) => set({ clientEngine: value }),
        reset: () => set(viewerPreferenceDefaults),
    }),
    {
      name: 'pacsleaf-viewer-preferences',
      storage: createJSONStorage(() => localStorage),
    },
  ),
)
