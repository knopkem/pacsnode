import { AlertTriangle, Loader2 } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'

import type { StreamingQuality } from '../../lib/runtime-config'
import {
  CanvasTransportClient,
  type StreamControlMessage,
  type StreamTransportKind,
} from '../../lib/streaming/canvas-transport'
import {
  createStreamSession,
  deleteStreamSession,
  supportsStreamingVideoDecoder,
} from '../../lib/streaming/streamer-session'

type StreamingViewportState =
  | { status: 'connecting'; message: string; sessionId?: string }
  | { status: 'ready'; message: string; sessionId: string }
  | { status: 'error'; message: string; sessionId?: string }

interface StreamingViewportProps {
  streamerUrl: string
  studyUid: string
  seriesUid: string
  quality: StreamingQuality
  layout: 'single' | 'quad'
  onUnavailable?: (message: string) => void
}

const SPECIAL_KEY_CODES: Record<string, number> = {
  ArrowDown: 0x28,
  ArrowLeft: 0x25,
  ArrowRight: 0x27,
  ArrowUp: 0x26,
  Enter: 0x0d,
  Escape: 0x1b,
  Tab: 0x09,
}

function clampNormalized(value: number): number {
  if (!Number.isFinite(value)) {
    return 0.5
  }

  return Math.min(1, Math.max(0, value))
}

function normalizedPointerPosition(event: PointerEvent, canvas: HTMLCanvasElement) {
  const rect = canvas.getBoundingClientRect()
  return {
    x: clampNormalized((event.clientX - rect.left) / rect.width),
    y: clampNormalized((event.clientY - rect.top) / rect.height),
  }
}

function normalizeKeyCode(event: KeyboardEvent): number | undefined {
  if (event.key.length === 1) {
    return event.key.toUpperCase().charCodeAt(0)
  }

  return SPECIAL_KEY_CODES[event.key]
}

function bindCanvasInputs(
  canvas: HTMLCanvasElement,
  getTransport: () => CanvasTransportClient | undefined,
) {
  const onPointerMove = (event: PointerEvent) => {
    const transport = getTransport()
    if (!transport) {
      return
    }

    const { x, y } = normalizedPointerPosition(event, canvas)
    transport.sendPointerMove(x, y, event.buttons, Math.round(event.timeStamp))
  }

  const onPointerDown = (event: PointerEvent) => {
    const transport = getTransport()
    if (!transport) {
      return
    }

    canvas.focus()
    const { x, y } = normalizedPointerPosition(event, canvas)
    transport.sendPointerDown(event.button, x, y)
    transport.sendPointerMove(x, y, event.buttons, Math.round(event.timeStamp))
  }

  const onPointerUp = (event: PointerEvent) => {
    const transport = getTransport()
    if (!transport) {
      return
    }

    const { x, y } = normalizedPointerPosition(event, canvas)
    transport.sendPointerUp(event.button, x, y)
  }

  const onWheel = (event: WheelEvent) => {
    const transport = getTransport()
    if (!transport) {
      return
    }

    event.preventDefault()
    transport.sendScroll(event.deltaX, event.deltaY)
  }

  const onKeyDown = (event: KeyboardEvent) => {
    const transport = getTransport()
    const code = transport ? normalizeKeyCode(event) : undefined
    if (transport && code !== undefined) {
      transport.sendKeyDown(code)
    }
  }

  const onKeyUp = (event: KeyboardEvent) => {
    const transport = getTransport()
    const code = transport ? normalizeKeyCode(event) : undefined
    if (transport && code !== undefined) {
      transport.sendKeyUp(code)
    }
  }

  const onContextMenu = (event: MouseEvent) => {
    event.preventDefault()
  }

  canvas.addEventListener('pointermove', onPointerMove)
  canvas.addEventListener('pointerdown', onPointerDown)
  canvas.addEventListener('pointerup', onPointerUp)
  canvas.addEventListener('wheel', onWheel, { passive: false })
  canvas.addEventListener('keydown', onKeyDown)
  canvas.addEventListener('keyup', onKeyUp)
  canvas.addEventListener('contextmenu', onContextMenu)

  return () => {
    canvas.removeEventListener('pointermove', onPointerMove)
    canvas.removeEventListener('pointerdown', onPointerDown)
    canvas.removeEventListener('pointerup', onPointerUp)
    canvas.removeEventListener('wheel', onWheel)
    canvas.removeEventListener('keydown', onKeyDown)
    canvas.removeEventListener('keyup', onKeyUp)
    canvas.removeEventListener('contextmenu', onContextMenu)
  }
}

export function StreamingViewport({
  streamerUrl,
  studyUid,
  seriesUid,
  quality,
  layout,
  onUnavailable,
}: StreamingViewportProps) {
  const canvasRef = useRef<HTMLCanvasElement | null>(null)
  const [state, setState] = useState<StreamingViewportState>({
    status: 'connecting',
    message: 'Bootstrapping streamed session…',
  })
  const [transportKind, setTransportKind] = useState<StreamTransportKind>('websocket')
  const [transportRttMs, setTransportRttMs] = useState<number | undefined>()

  useEffect(() => {
    const canvas = canvasRef.current
    if (!canvas) {
      return undefined
    }

    let disposed = false
    let activeSessionId: string | undefined
    let transport: CanvasTransportClient | undefined

    const resizeCanvas = () => {
      const container = canvas.parentElement
      if (!container) return
      const rect = container.getBoundingClientRect()
      const w = Math.max(Math.round(rect.width), 1)
      const h = Math.max(Math.round(rect.height), 1)
      if (canvas.width !== w || canvas.height !== h) {
        canvas.width = w
        canvas.height = h
      }
    }

    resizeCanvas()
    const resizeObserver = new ResizeObserver(resizeCanvas)
    resizeObserver.observe(canvas.parentElement ?? canvas)
    const unbindInputs = bindCanvasInputs(canvas, () => transport)

    void (async () => {
      const supportsVideoDecoder = await supportsStreamingVideoDecoder()
      const session = await createStreamSession(streamerUrl, {
        studyUid,
        seriesUid,
        quality,
        layout,
        supportsVideoDecoder,
      })

      if (disposed) {
        await deleteStreamSession(streamerUrl, session.sessionId)
        return
      }

      activeSessionId = session.sessionId
      setState({
        status: 'connecting',
        message: 'Connecting to streamed viewport…',
        sessionId: session.sessionId,
      })
      setTransportKind(session.transport)
      setTransportRttMs(undefined)

      transport = new CanvasTransportClient({
        websocketUrl: session.websocketUrl,
        webtransport: session.webtransport,
        canvas,
        onStatus: () => {},
        onTransportChange: (kind) => setTransportKind(kind),
        onControlMessage: (message: StreamControlMessage) => {
          if (message.type === 'session-metrics') {
            if (
              typeof message.transportRttMs === 'number' &&
              Number.isFinite(message.transportRttMs)
            ) {
              setTransportRttMs(message.transportRttMs)
            }
          }
        },
      })
      await transport.connect()

      if (disposed) {
        transport.disconnect()
        await deleteStreamSession(streamerUrl, session.sessionId)
        return
      }

      setState({
        status: 'ready',
        message: 'Streaming connected',
        sessionId: session.sessionId,
      })
    })().catch(async (error: unknown) => {
      const message =
        error instanceof Error ? error.message : 'The streaming session could not be opened.'

      if (activeSessionId) {
        deleteStreamSession(streamerUrl, activeSessionId).catch((releaseError: unknown) => {
          console.warn('Failed to release streaming session', releaseError)
        })
      }

      if (!disposed) {
        onUnavailable?.(message)
        setState({
          status: 'error',
          message,
          sessionId: activeSessionId,
        })
      }
    })

    return () => {
      disposed = true
      resizeObserver.disconnect()
      unbindInputs()
      transport?.disconnect()

      if (activeSessionId) {
        deleteStreamSession(streamerUrl, activeSessionId).catch((error: unknown) => {
          console.warn('Failed to release streaming session', error)
        })
      }
    }
  }, [layout, onUnavailable, quality, seriesUid, streamerUrl, studyUid])

  return (
    <div className="absolute inset-0 bg-black">
      <canvas
        ref={canvasRef}
        tabIndex={0}
        className="absolute inset-0 touch-none bg-black outline-none"
        style={{ width: '100%', height: '100%', display: 'block' }}
      />

      {state.status === 'connecting' ? (
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

      {state.status === 'ready' ? (
        <div className="pointer-events-none absolute bottom-1 left-1 rounded bg-black/60 px-1.5 py-0.5 text-[10px] text-slate-400 backdrop-blur-sm">
          Streaming · {quality} ·{' '}
          <span
            className={
              transportKind === 'webtransport' ? 'text-emerald-400' : 'text-amber-400'
            }
          >
            {transportKind === 'webtransport' ? 'WebTransport' : 'WebSocket (fallback)'}
          </span>
          {transportRttMs !== undefined ? ` · ${transportRttMs.toFixed(0)}ms` : ''}
        </div>
      ) : null}
    </div>
  )
}
