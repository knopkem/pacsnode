import clsx from 'clsx'
import { AlertTriangle, Loader2, RotateCcw } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'

import {
  mountStackViewport,
  type StackPrimaryTool,
  type StackViewportSnapshot,
} from '../../lib/cornerstone'

type StackViewportState = StackViewportSnapshot & {
  message?: string
  status: 'loading' | 'ready' | 'error'
}

interface StackViewportProps {
  studyUid: string
  seriesUid: string
}

const STACK_TOOLS: Array<{ label: string; tool: StackPrimaryTool }> = [
  { label: 'W/L', tool: 'windowLevel' },
  { label: 'Pan', tool: 'pan' },
  { label: 'Zoom', tool: 'zoom' },
  { label: 'Length', tool: 'length' },
  { label: 'Angle', tool: 'angle' },
  { label: 'Rect', tool: 'rectangleRoi' },
  { label: 'Ellipse', tool: 'ellipticalRoi' },
]

export function StackViewport({ studyUid, seriesUid }: StackViewportProps) {
  const elementRef = useRef<HTMLDivElement | null>(null)
  const renderingEngineIdRef = useRef(`pacsleaf-rendering-engine-${crypto.randomUUID()}`)
  const viewportIdRef = useRef(`pacsleaf-stack-viewport-${crypto.randomUUID()}`)
  const controllerRef = useRef<Awaited<ReturnType<typeof mountStackViewport>> | null>(null)
  const [state, setState] = useState<StackViewportState>({
    activeTool: 'windowLevel',
    currentImageIndex: 0,
    imageCount: 0,
    measurementCount: 0,
    message: undefined,
    status: 'loading',
  })

  useEffect(() => {
    const element = elementRef.current
    if (!element) return undefined

    const abortController = new AbortController()
    let disposed = false
    let unsubscribe = () => {}
    let cleanup = () => {}

    void mountStackViewport({
      element,
      studyUid,
      seriesUid,
      renderingEngineId: renderingEngineIdRef.current,
      viewportId: viewportIdRef.current,
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
      <div ref={elementRef} className="absolute inset-0" />

      {/* Compact toolbar overlay */}
      <div className="absolute inset-x-0 top-0 z-10 flex items-center gap-1 bg-black/60 px-2 py-1 backdrop-blur-sm">
        {STACK_TOOLS.map(({ label, tool }) => (
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
        <div className="ml-auto flex items-center gap-2">
          {state.imageCount > 0 ? (
            <span className="text-[10px] text-slate-400 tabular-nums">
              {state.currentImageIndex + 1}/{state.imageCount}
            </span>
          ) : null}
          <button
            type="button"
            disabled={!ready}
            onClick={() => controllerRef.current?.reset()}
            className={clsx(
              'toolbar-button toolbar-button-inactive',
              !ready && 'cursor-not-allowed opacity-40',
            )}
          >
            <RotateCcw className="h-3 w-3" />
          </button>
        </div>
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
