import { RotateCcw } from 'lucide-react'

import { Toggle } from '../components/ui/Toggle'
import { useViewerPreferencesStore } from '../state/preferences'

export function SettingsPage() {
  const defaultStudyLimit = useViewerPreferencesStore((s) => s.defaultStudyLimit)
  const autoSelectFirstSeries = useViewerPreferencesStore((s) => s.autoSelectFirstSeries)
  const preferredRenderingMode = useViewerPreferencesStore((s) => s.preferredRenderingMode)
  const streamingQuality = useViewerPreferencesStore((s) => s.streamingQuality)
  const viewportLayout = useViewerPreferencesStore((s) => s.viewportLayout)
  const setDefaultStudyLimit = useViewerPreferencesStore((s) => s.setDefaultStudyLimit)
  const setAutoSelectFirstSeries = useViewerPreferencesStore((s) => s.setAutoSelectFirstSeries)
  const setPreferredRenderingMode = useViewerPreferencesStore((s) => s.setPreferredRenderingMode)
  const setStreamingQuality = useViewerPreferencesStore((s) => s.setStreamingQuality)
  const setViewportLayout = useViewerPreferencesStore((s) => s.setViewportLayout)
  const reset = useViewerPreferencesStore((s) => s.reset)

  return (
    <div className="mx-auto max-w-2xl space-y-6 px-4 py-6">
      <div className="flex items-center justify-between">
        <h1 className="text-lg font-semibold text-white">Settings</h1>
        <button
          type="button"
          onClick={reset}
          className="inline-flex items-center gap-1.5 rounded border border-slate-700 bg-slate-900 px-2.5 py-1.5 text-xs font-medium text-slate-300 transition hover:bg-slate-800 hover:text-white"
        >
          <RotateCcw className="h-3 w-3" />
          Reset defaults
        </button>
      </div>

      <section className="space-y-3">
        <h2 className="text-xs font-semibold uppercase tracking-wider text-slate-500">General</h2>
        <div className="grid gap-3 sm:grid-cols-2">
          <div>
            <label className="text-xs text-slate-400" htmlFor="studyLimit">
              Default study page size
            </label>
            <input
              id="studyLimit"
              type="number"
              min={10}
              max={250}
              className="field mt-1"
              value={defaultStudyLimit}
              onChange={(e) => setDefaultStudyLimit(Number.parseInt(e.target.value, 10))}
            />
          </div>
          <div>
            <label className="text-xs text-slate-400" htmlFor="layout">
              Default viewport layout
            </label>
            <select
              id="layout"
              className="select-field mt-1"
              value={viewportLayout}
              onChange={(e) => setViewportLayout(e.target.value === 'quad' ? 'quad' : 'single')}
            >
              <option value="single">Single viewport</option>
              <option value="quad">Quad MPR + 3D</option>
            </select>
          </div>
        </div>
        <Toggle
          checked={autoSelectFirstSeries}
          onCheckedChange={setAutoSelectFirstSeries}
          label="Auto-select first series"
          compact
        />
      </section>

      <section className="space-y-3">
        <h2 className="text-xs font-semibold uppercase tracking-wider text-slate-500">Rendering</h2>
        <div className="grid gap-3 sm:grid-cols-2">
          <div>
            <label className="text-xs text-slate-400" htmlFor="renderMode">
              Rendering mode
            </label>
            <select
              id="renderMode"
              className="select-field mt-1"
              value={preferredRenderingMode}
              onChange={(e) =>
                setPreferredRenderingMode(e.target.value === 'streaming' ? 'streaming' : 'client')
              }
            >
              <option value="streaming">Streaming (server-side)</option>
              <option value="client">Client-side</option>
            </select>
            <p className="mt-1 text-[11px] text-slate-500">
              Streaming renders on the server and sends frames to the browser. Client-side decodes DICOM data locally.
            </p>
          </div>
          <div>
            <label className="text-xs text-slate-400" htmlFor="quality">
              Streaming quality
            </label>
            <select
              id="quality"
              className="select-field mt-1"
              value={streamingQuality}
              onChange={(e) =>
                setStreamingQuality(
                  e.target.value === 'diagnostic'
                    ? 'diagnostic'
                    : e.target.value === 'mobile'
                      ? 'mobile'
                      : 'balanced',
                )
              }
            >
              <option value="diagnostic">Diagnostic</option>
              <option value="balanced">Balanced</option>
              <option value="mobile">Mobile</option>
            </select>
          </div>
        </div>
      </section>
    </div>
  )
}
