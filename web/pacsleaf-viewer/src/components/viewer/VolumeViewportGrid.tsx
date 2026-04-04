import clsx from 'clsx'
import { AlertTriangle, Loader2, RotateCcw } from 'lucide-react'
import { type RefObject, useEffect, useMemo, useRef, useState } from 'react'

import {
  mountVolumeViewportGrid,
  type MprPrimaryTool,
  type VolumeViewportSnapshot,
} from '../../lib/cornerstone'

type VolumeViewportState = VolumeViewportSnapshot & {
  message?: string
  status: 'loading' | 'ready' | 'error'
}

interface VolumeViewportGridProps {
  studyUid: string
  seriesUid: string
  modality?: string
}

interface ViewportPanelProps {
  label: string
  viewportRef: RefObject<HTMLDivElement | null>
}

function ViewportPanel({ label, viewportRef }: ViewportPanelProps) {
  return (
    <div className="relative min-h-0 overflow-hidden bg-black">
      <div ref={viewportRef} className="absolute inset-0" />
      <div className="pointer-events-none absolute bottom-1 left-1 rounded bg-black/60 px-1.5 py-0.5 text-[10px] text-slate-300 backdrop-blur-sm">
        {label}
      </div>
    </div>
  )
}

function preferredVolumePreset(modality?: string): string | undefined {
  const n = modality?.toUpperCase()
  if (n?.startsWith('CT')) return 'CT-Bone'
  if (n?.startsWith('MR')) return 'MR-Default'
  return undefined
}

function volumePresetOptions(modality?: string): string[] {
  const n = modality?.toUpperCase()
  if (n?.startsWith('CT')) return ['CT-Bone', 'CT-Soft-Tissue', 'CT-Lung', 'CT-MIP']
  if (n?.startsWith('MR')) return ['MR-Default', 'MR-Angio', 'MR-T2-Brain', 'MR-MIP']
  return []
}

const MPR_TOOLS: Array<{ label: string; tool: MprPrimaryTool }> = [
  { label: 'Crosshairs', tool: 'crosshairs' },
  { label: 'W/L', tool: 'windowLevel' },
  { label: 'Pan', tool: 'pan' },
  { label: 'Zoom', tool: 'zoom' },
  { label: 'Length', tool: 'length' },
  { label: 'Angle', tool: 'angle' },
  { label: 'Rect', tool: 'rectangleRoi' },
  { label: 'Ellipse', tool: 'ellipticalRoi' },
]

export function VolumeViewportGrid({
  studyUid,
  seriesUid,
  modality,
}: VolumeViewportGridProps) {
  const axialRef = useRef<HTMLDivElement | null>(null)
  const coronalRef = useRef<HTMLDivElement | null>(null)
  const sagittalRef = useRef<HTMLDivElement | null>(null)
  const volumeRef = useRef<HTMLDivElement | null>(null)
  const renderingEngineIdRef = useRef(`pacsleaf-volume-engine-${crypto.randomUUID()}`)
  const controllerRef = useRef<Awaited<ReturnType<typeof mountVolumeViewportGrid>> | null>(null)
  const presetOptions = useMemo(() => volumePresetOptions(modality), [modality])
  const initialPreset = useMemo(() => preferredVolumePreset(modality), [modality])
  const [state, setState] = useState<VolumeViewportState>({
    activeTool: 'crosshairs',
    status: 'loading',
    imageCount: 0,
    measurementCount: 0,
    volumePreset: initialPreset,
  })
  const [selectedPreset, setSelectedPreset] = useState<string>(initialPreset ?? '')

  useEffect(() => {
    const axial = axialRef.current
    const coronal = coronalRef.current
    const sagittal = sagittalRef.current
    const volume = volumeRef.current
    if (!axial || !coronal || !sagittal || !volume) return undefined

    const abortController = new AbortController()
    let disposed = false
    let unsubscribe = () => {}
    let cleanup = () => {}

    void mountVolumeViewportGrid({
      elements: { axial, coronal, sagittal, volume },
      studyUid,
      seriesUid,
      renderingEngineId: renderingEngineIdRef.current,
      signal: abortController.signal,
      volumePreset: initialPreset,
    })
      .then((controller) => {
        if (disposed) {
          controller.destroy()
          return
        }
        controllerRef.current = controller
        unsubscribe = controller.subscribe((snapshot) => {
          setState({ ...snapshot, message: undefined, status: 'ready' })
        })
        cleanup = () => {
          unsubscribe()
          controllerRef.current = null
          controller.destroy()
        }
      })
      .catch((error: unknown) => {
        if (disposed || abortController.signal.aborted) return
        controllerRef.current = null
        setState((prev) => ({
          ...prev,
          imageCount: 0,
          message: error instanceof Error ? error.message : 'Failed to load volume.',
          status: 'error',
        }))
      })

    return () => {
      disposed = true
      abortController.abort()
      cleanup()
    }
  }, [initialPreset, seriesUid, studyUid])

  useEffect(() => {
    controllerRef.current?.setVolumePreset(selectedPreset || undefined)
  }, [selectedPreset])

  const ready = state.status === 'ready'

  return (
    <div className="absolute inset-0 bg-black">
      <div className="absolute inset-0 grid grid-cols-2 grid-rows-2 gap-px bg-slate-900">
        <ViewportPanel label="Axial" viewportRef={axialRef} />
        <ViewportPanel label="Coronal" viewportRef={coronalRef} />
        <ViewportPanel label="Sagittal" viewportRef={sagittalRef} />
        <ViewportPanel label="3D" viewportRef={volumeRef} />
      </div>

      {/* Compact toolbar overlay */}
      <div className="absolute inset-x-0 top-0 flex items-center gap-1 bg-black/60 px-2 py-1 backdrop-blur-sm">
        {MPR_TOOLS.map(({ label, tool }) => (
          <button
            key={tool}
            type="button"
            disabled={!ready}
            onClick={() => controllerRef.current?.setPrimaryTool(tool)}
            className={clsx(
              'toolbar-button',
              state.activeTool === tool ? 'toolbar-button-active' : 'toolbar-button-inactive',
              !ready && 'cursor-not-allowed opacity-40',
            )}
          >
            {label}
          </button>
        ))}

        {presetOptions.length > 0 ? (
          <select
            value={selectedPreset}
            disabled={!ready}
            onChange={(e) => setSelectedPreset(e.target.value)}
            className={clsx(
              'ml-auto h-7 rounded border border-slate-700 bg-slate-900 px-1.5 text-xs text-slate-200',
              !ready && 'opacity-40',
            )}
          >
            {presetOptions.map((p) => (
              <option key={p} value={p}>{p}</option>
            ))}
          </select>
        ) : null}

        <button
          type="button"
          disabled={!ready}
          onClick={() => controllerRef.current?.reset()}
          className={clsx(
            'toolbar-button toolbar-button-inactive',
            presetOptions.length === 0 && 'ml-auto',
            !ready && 'cursor-not-allowed opacity-40',
          )}
        >
          <RotateCcw className="h-3 w-3" />
        </button>
      </div>

      {state.status === 'loading' ? (
        <div className="absolute inset-0 flex items-center justify-center bg-black/70">
          <Loader2 className="h-5 w-5 animate-spin text-slate-400" />
        </div>
      ) : null}

      {state.status === 'error' ? (
        <div className="absolute inset-0 flex items-center justify-center bg-black/80 px-6">
          <div className="flex items-center gap-2 text-sm text-rose-300">
            <AlertTriangle className="h-4 w-4" />
            {state.message}
          </div>
        </div>
      ) : null}
    </div>
  )
}
