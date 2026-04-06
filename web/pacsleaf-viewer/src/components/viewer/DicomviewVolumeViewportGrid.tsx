import { AlertTriangle, Loader2 } from 'lucide-react'
import { type RefObject, useEffect, useRef, useState } from 'react'

import {
  mountDicomviewVolumeGrid,
  type DicomviewVolumeController,
  type DicomviewVolumeSnapshot,
} from '../../lib/engines/dicomview/volume'
import type { MprPrimaryTool } from '../../lib/engines/types'
import { type ToolDefinition, ViewportToolbar } from './ViewportToolbar'

type VolumeViewportState = DicomviewVolumeSnapshot & {
  message?: string
  status: 'loading' | 'ready' | 'error'
}

interface DicomviewVolumeViewportGridProps {
  studyUid: string
  seriesUid: string
  modality?: string
}

interface ViewportPanelProps {
  label: string
  canvasRef: RefObject<HTMLCanvasElement | null>
}

function ViewportPanel({ label, canvasRef }: ViewportPanelProps) {
  return (
    <div className="relative min-h-0 overflow-hidden bg-black">
      <canvas
        ref={canvasRef}
        className="absolute inset-0"
        style={{ width: '100%', height: '100%', display: 'block' }}
      />
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

const MPR_TOOLS: ToolDefinition[] = [
  { label: 'Crosshairs', tool: 'crosshairs' },
  { label: 'W/L', tool: 'windowLevel' },
  { label: 'Pan', tool: 'pan' },
  { label: 'Zoom', tool: 'zoom' },
  { label: 'Length', tool: 'length' },
  { label: 'Angle', tool: 'angle' },
  { label: 'Rect', tool: 'rectangleRoi' },
  { label: 'Ellipse', tool: 'ellipticalRoi' },
]

export function DicomviewVolumeViewportGrid({
  studyUid,
  seriesUid,
  modality,
}: DicomviewVolumeViewportGridProps) {
  const axialRef = useRef<HTMLCanvasElement | null>(null)
  const coronalRef = useRef<HTMLCanvasElement | null>(null)
  const sagittalRef = useRef<HTMLCanvasElement | null>(null)
  const volumeRef = useRef<HTMLCanvasElement | null>(null)
  const controllerRef = useRef<DicomviewVolumeController | null>(null)
  const presetOptions = volumePresetOptions(modality)
  const initialPreset = preferredVolumePreset(modality)
  const [state, setState] = useState<VolumeViewportState>({
    activeTool: 'crosshairs',
    status: 'loading',
    imageCount: 0,
    loadedCount: 0,
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

    void mountDicomviewVolumeGrid({
      elements: { axial, coronal, sagittal, volume },
      studyUid,
      seriesUid,
      volumePreset: initialPreset,
      signal: abortController.signal,
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
  const progressPercent =
    state.imageCount > 0
      ? Math.round((state.loadedCount / state.imageCount) * 100)
      : 0

  return (
    <div className="absolute inset-0 bg-black">
      <div className="absolute inset-0 grid grid-cols-2 grid-rows-2 gap-px bg-slate-900">
        <ViewportPanel label="Axial" canvasRef={axialRef} />
        <ViewportPanel label="Coronal" canvasRef={coronalRef} />
        <ViewportPanel label="Sagittal" canvasRef={sagittalRef} />
        <ViewportPanel label="3D" canvasRef={volumeRef} />
      </div>

      <ViewportToolbar
        tools={MPR_TOOLS}
        activeTool={state.activeTool}
        onToolChange={(tool) =>
          controllerRef.current?.setPrimaryTool(tool as MprPrimaryTool)
        }
        onReset={() => controllerRef.current?.reset()}
        ready={ready}
        annotationsDisabled
        presetOptions={presetOptions}
        selectedPreset={selectedPreset}
        onPresetChange={setSelectedPreset}
      />

      {state.status === 'loading' ? (
        <div className="absolute inset-0 flex flex-col items-center justify-center bg-black/70">
          <Loader2 className="h-5 w-5 animate-spin text-slate-400" />
          {state.imageCount > 0 ? (
            <div className="mt-2 w-40">
              <div className="h-1 rounded-full bg-slate-700">
                <div
                  className="h-1 rounded-full bg-sky-500 transition-all"
                  style={{ width: `${progressPercent}%` }}
                />
              </div>
              <p className="mt-1 text-center text-[10px] text-slate-500">
                {state.loadedCount}/{state.imageCount} slices
              </p>
            </div>
          ) : null}
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
