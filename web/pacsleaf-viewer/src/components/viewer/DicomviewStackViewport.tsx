import { AlertTriangle, Loader2 } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'

import {
  mountDicomviewStack,
  type DicomviewStackController,
  type DicomviewStackSnapshot,
} from '../../lib/engines/dicomview/stack'
import type { StackPrimaryTool } from '../../lib/engines/types'
import { type ToolDefinition, ViewportToolbar } from './ViewportToolbar'

type StackViewportState = DicomviewStackSnapshot & {
  message?: string
  status: 'loading' | 'ready' | 'error'
}

interface DicomviewStackViewportProps {
  studyUid: string
  seriesUid: string
}

const STACK_TOOLS: ToolDefinition[] = [
  { label: 'W/L', tool: 'windowLevel' },
  { label: 'Pan', tool: 'pan' },
  { label: 'Zoom', tool: 'zoom' },
  { label: 'Length', tool: 'length' },
  { label: 'Angle', tool: 'angle' },
  { label: 'Rect', tool: 'rectangleRoi' },
  { label: 'Ellipse', tool: 'ellipticalRoi' },
]

export function DicomviewStackViewport({
  studyUid,
  seriesUid,
}: DicomviewStackViewportProps) {
  const canvasRef = useRef<HTMLCanvasElement | null>(null)
  const controllerRef = useRef<DicomviewStackController | null>(null)
  const [state, setState] = useState<StackViewportState>({
    activeTool: 'windowLevel',
    currentImageIndex: 0,
    imageCount: 0,
    message: undefined,
    status: 'loading',
  })

  useEffect(() => {
    const canvas = canvasRef.current
    if (!canvas) return undefined

    const abortController = new AbortController()
    let disposed = false
    let unsubscribe = () => {}
    let cleanup = () => {}

    void mountDicomviewStack({
      canvas,
      studyUid,
      seriesUid,
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
          message: error instanceof Error ? error.message : 'Failed to load stack.',
          status: 'error',
        }))
      })

    return () => {
      disposed = true
      abortController.abort()
      cleanup()
    }
  }, [seriesUid, studyUid])

  const ready = state.status === 'ready'

  return (
    <div className="absolute inset-0 bg-black">
      <canvas
        ref={canvasRef}
        className="absolute inset-0"
        style={{ width: '100%', height: '100%', display: 'block' }}
      />

      <ViewportToolbar
        tools={STACK_TOOLS}
        activeTool={state.activeTool}
        onToolChange={(tool) =>
          controllerRef.current?.setPrimaryTool(tool as StackPrimaryTool)
        }
        onReset={() => controllerRef.current?.reset()}
        ready={ready}
        annotationsDisabled
        rightContent={
          state.imageCount > 0 ? (
            <span className="text-[10px] text-slate-400 tabular-nums">
              {state.currentImageIndex + 1}/{state.imageCount}
            </span>
          ) : null
        }
      />

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
