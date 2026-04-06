import type { StackViewer, Viewer } from '@knopkem/dicomview'

import type { MprPrimaryTool, StackPrimaryTool } from '../types'

type ActiveTool = StackPrimaryTool | MprPrimaryTool

interface DragState {
  startX: number
  startY: number
  lastX: number
  lastY: number
  button: number
}

type ViewportAwareViewer = Viewer & {
  setCrosshairFromViewport(
    viewport: 'axial' | 'coronal' | 'sagittal',
    u: number,
    v: number,
    width: number,
    height: number,
  ): void
}

const WL_SENSITIVITY = 1.0
const PAN_SENSITIVITY = 1.0
const ZOOM_SENSITIVITY = 0.005

/**
 * Binds mouse/pointer/wheel events on an MPR canvas and translates
 * them into calls on the dicomview Viewer API based on the currently
 * active tool.
 *
 * Returns a cleanup function that removes all listeners.
 */
export function bindDicomviewInputs(
  canvas: HTMLCanvasElement,
  getViewer: () => Viewer | undefined,
  getActiveTool: () => ActiveTool,
  options?: {
    viewportIndex?: 'axial' | 'coronal' | 'sagittal'
    onWindowLevelChange?: (center: number, width: number) => void
  },
): () => void {
  let drag: DragState | null = null
  let currentWLCenter = 400
  let currentWLWidth = 1500

  const VIEWPORT_MAP: Record<string, 'axial' | 'coronal' | 'sagittal'> = {
    axial: 'axial',
    coronal: 'coronal',
    sagittal: 'sagittal',
  }
  const viewportId = VIEWPORT_MAP[options?.viewportIndex ?? 'axial'] ?? 'axial'

  function onPointerDown(event: PointerEvent) {
    if (event.button === 2) {
      event.preventDefault()
    }
    canvas.setPointerCapture(event.pointerId)
    drag = {
      startX: event.clientX,
      startY: event.clientY,
      lastX: event.clientX,
      lastY: event.clientY,
      button: event.button,
    }
  }

  function onPointerMove(event: PointerEvent) {
    if (!drag) return
    const viewer = getViewer()
    if (!viewer) return

    const dx = event.clientX - drag.lastX
    const dy = event.clientY - drag.lastY
    drag.lastX = event.clientX
    drag.lastY = event.clientY

    const tool = getActiveTool()

    if (drag.button === 0) {
      switch (tool) {
        case 'windowLevel': {
          currentWLCenter += dy * WL_SENSITIVITY
          currentWLWidth += dx * WL_SENSITIVITY
          currentWLWidth = Math.max(1, currentWLWidth)
          viewer.setWindowLevel(currentWLCenter, currentWLWidth)
          options?.onWindowLevelChange?.(currentWLCenter, currentWLWidth)
          viewer.render()
          break
        }
        case 'pan': {
          viewer.pan(dx * PAN_SENSITIVITY, dy * PAN_SENSITIVITY)
          viewer.render()
          break
        }
        case 'zoom': {
          const factor = 1 + dy * ZOOM_SENSITIVITY
          viewer.zoom(factor)
          viewer.render()
          break
        }
        case 'crosshairs': {
          const rect = canvas.getBoundingClientRect()
          const nx = (event.clientX - rect.left) / rect.width
          const ny = (event.clientY - rect.top) / rect.height
          ;(viewer as ViewportAwareViewer).setCrosshairFromViewport(
            viewportId,
            nx,
            ny,
            rect.width,
            rect.height,
          )
          viewer.render()
          break
        }
        default:
          break
      }
    } else if (drag.button === 1) {
      viewer.pan(dx * PAN_SENSITIVITY, dy * PAN_SENSITIVITY)
      viewer.render()
    } else if (drag.button === 2) {
      viewer.zoom(1 + dy * ZOOM_SENSITIVITY)
      viewer.render()
    }
  }

  function onPointerUp(event: PointerEvent) {
    if (drag) {
      canvas.releasePointerCapture(event.pointerId)
      drag = null
    }
  }

  function onWheel(event: WheelEvent) {
    event.preventDefault()
    const viewer = getViewer()
    if (!viewer) return

    const delta = event.deltaY > 0 ? 1 : event.deltaY < 0 ? -1 : 0
    if (delta !== 0) {
      viewer.scrollSlice(viewportId, delta)
      viewer.render()
    }
  }

  function onContextMenu(event: MouseEvent) {
    event.preventDefault()
  }

  canvas.addEventListener('pointerdown', onPointerDown)
  canvas.addEventListener('pointermove', onPointerMove)
  canvas.addEventListener('pointerup', onPointerUp)
  canvas.addEventListener('wheel', onWheel, { passive: false })
  canvas.addEventListener('contextmenu', onContextMenu)

  return () => {
    canvas.removeEventListener('pointerdown', onPointerDown)
    canvas.removeEventListener('pointermove', onPointerMove)
    canvas.removeEventListener('pointerup', onPointerUp)
    canvas.removeEventListener('wheel', onWheel)
    canvas.removeEventListener('contextmenu', onContextMenu)
  }
}

/**
 * Binds mouse/pointer/wheel events for a StackViewer (single canvas).
 *
 * StackViewer supports window/level and scroll. Pan/zoom are not yet
 * available — those tools are silently ignored when selected.
 */
export function bindStackInputs(
  canvas: HTMLCanvasElement,
  getViewer: () => StackViewer | undefined,
  getActiveTool: () => StackPrimaryTool,
  options?: {
    onWindowLevelChange?: (center: number, width: number) => void
  },
): () => void {
  let drag: DragState | null = null
  let currentWLCenter = 400
  let currentWLWidth = 1500

  function onPointerDown(event: PointerEvent) {
    if (event.button === 2) event.preventDefault()
    canvas.setPointerCapture(event.pointerId)
    drag = {
      startX: event.clientX,
      startY: event.clientY,
      lastX: event.clientX,
      lastY: event.clientY,
      button: event.button,
    }
  }

  function onPointerMove(event: PointerEvent) {
    if (!drag) return
    const viewer = getViewer()
    if (!viewer) return

    const dx = event.clientX - drag.lastX
    const dy = event.clientY - drag.lastY
    drag.lastX = event.clientX
    drag.lastY = event.clientY

    // Left button: active tool
    if (drag.button === 0) {
      const tool = getActiveTool()
      if (tool === 'windowLevel') {
        currentWLCenter += dy * WL_SENSITIVITY
        currentWLWidth += dx * WL_SENSITIVITY
        currentWLWidth = Math.max(1, currentWLWidth)
        viewer.setWindowLevel(currentWLCenter, currentWLWidth)
        options?.onWindowLevelChange?.(currentWLCenter, currentWLWidth)
        viewer.render()
      }
      // pan/zoom not available on StackViewer — silently ignored
    }
  }

  function onPointerUp(event: PointerEvent) {
    if (drag) {
      canvas.releasePointerCapture(event.pointerId)
      drag = null
    }
  }

  function onWheel(event: WheelEvent) {
    event.preventDefault()
    const viewer = getViewer()
    if (!viewer) return

    const delta = event.deltaY > 0 ? 1 : event.deltaY < 0 ? -1 : 0
    if (delta !== 0) {
      viewer.scrollSlice(delta)
      viewer.render()
    }
  }

  function onContextMenu(event: MouseEvent) {
    event.preventDefault()
  }

  canvas.addEventListener('pointerdown', onPointerDown)
  canvas.addEventListener('pointermove', onPointerMove)
  canvas.addEventListener('pointerup', onPointerUp)
  canvas.addEventListener('wheel', onWheel, { passive: false })
  canvas.addEventListener('contextmenu', onContextMenu)

  return () => {
    canvas.removeEventListener('pointerdown', onPointerDown)
    canvas.removeEventListener('pointermove', onPointerMove)
    canvas.removeEventListener('pointerup', onPointerUp)
    canvas.removeEventListener('wheel', onWheel)
    canvas.removeEventListener('contextmenu', onContextMenu)
  }
}

/**
 * Bind trackball-style orbit interaction on the volume canvas.
 * Left-drag = orbit, middle-drag = pan, right-drag = zoom, wheel = zoom.
 */
export function bindVolumeCanvasInputs(
  canvas: HTMLCanvasElement,
  getViewer: () => Viewer | undefined,
): () => void {
  let drag: DragState | null = null

  function onPointerDown(event: PointerEvent) {
    if (event.button === 2) event.preventDefault()
    canvas.setPointerCapture(event.pointerId)
    drag = {
      startX: event.clientX,
      startY: event.clientY,
      lastX: event.clientX,
      lastY: event.clientY,
      button: event.button,
    }
  }

  function onPointerMove(event: PointerEvent) {
    if (!drag) return
    const viewer = getViewer()
    if (!viewer) return

    const dx = event.clientX - drag.lastX
    const dy = event.clientY - drag.lastY
    drag.lastX = event.clientX
    drag.lastY = event.clientY

    if (drag.button === 0) {
      viewer.orbit(dx, dy)
      viewer.render()
    } else if (drag.button === 1) {
      viewer.pan(dx * PAN_SENSITIVITY, dy * PAN_SENSITIVITY)
      viewer.render()
    } else if (drag.button === 2) {
      viewer.zoom(1 + dy * ZOOM_SENSITIVITY)
      viewer.render()
    }
  }

  function onPointerUp(event: PointerEvent) {
    if (drag) {
      canvas.releasePointerCapture(event.pointerId)
      drag = null
    }
  }

  function onWheel(event: WheelEvent) {
    event.preventDefault()
    const viewer = getViewer()
    if (!viewer) return

    const factor = event.deltaY > 0 ? 1.05 : 0.95
    viewer.zoom(factor)
    viewer.render()
  }

  function onContextMenu(event: MouseEvent) {
    event.preventDefault()
  }

  canvas.addEventListener('pointerdown', onPointerDown)
  canvas.addEventListener('pointermove', onPointerMove)
  canvas.addEventListener('pointerup', onPointerUp)
  canvas.addEventListener('wheel', onWheel, { passive: false })
  canvas.addEventListener('contextmenu', onContextMenu)

  return () => {
    canvas.removeEventListener('pointerdown', onPointerDown)
    canvas.removeEventListener('pointermove', onPointerMove)
    canvas.removeEventListener('pointerup', onPointerUp)
    canvas.removeEventListener('wheel', onWheel)
    canvas.removeEventListener('contextmenu', onContextMenu)
  }
}
