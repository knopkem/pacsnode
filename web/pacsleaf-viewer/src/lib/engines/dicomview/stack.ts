import { DICOMwebLoader, StackViewer } from '@knopkem/dicomview'

import { getRuntimeConfig } from '../../runtime-config'
import type { StackPrimaryTool } from '../types'
import { ensureDicomviewReady } from './init'
import { bindStackInputs } from './mouse-handler'

type SnapshotListener = (snapshot: DicomviewStackSnapshot) => void

export interface DicomviewStackSnapshot {
  activeTool: StackPrimaryTool
  currentImageIndex: number
  imageCount: number
}

export interface DicomviewStackController {
  destroy(): void
  getSnapshot(): DicomviewStackSnapshot
  reset(): void
  setPrimaryTool(tool: StackPrimaryTool): void
  subscribe(listener: SnapshotListener): () => void
}

/**
 * Mount a dicomview-backed stack viewport using the dedicated StackViewer.
 *
 * Since dicomview 0.2.0 the StackViewer only needs a single canvas —
 * no hidden off-screen canvases required.
 */
export async function mountDicomviewStack(params: {
  canvas: HTMLCanvasElement
  studyUid: string
  seriesUid: string
  signal?: AbortSignal
}): Promise<DicomviewStackController> {
  await ensureDicomviewReady()

  if (params.signal?.aborted) {
    throw new DOMException('The stack request was aborted.', 'AbortError')
  }

  const viewer = await StackViewer.create({ canvas: params.canvas })

  if (params.signal?.aborted) {
    viewer.destroy()
    throw new DOMException('The stack request was aborted.', 'AbortError')
  }

  let activeTool: StackPrimaryTool = 'windowLevel'
  let imageCount = 0
  let destroyed = false
  const listeners = new Set<SnapshotListener>()

  const unbindInputs = bindStackInputs(
    params.canvas,
    () => (destroyed ? undefined : viewer),
    () => activeTool,
  )

  const getSnapshot = (): DicomviewStackSnapshot => ({
    activeTool,
    currentImageIndex: Math.round(viewer.loadingProgress * Math.max(imageCount - 1, 0)),
    imageCount,
  })

  const emitSnapshot = () => {
    const snapshot = getSnapshot()
    for (const listener of listeners) {
      listener(snapshot)
    }
  }

  // Load series
  const runtimeConfig = getRuntimeConfig()
  const wadoRoot = new URL(
    runtimeConfig.dicomweb.wadoRoot,
    window.location.origin,
  ).toString()

  const loader = new DICOMwebLoader({ wadoRoot, renderDuringLoad: true })
  loader.onProgress((_loaded, total) => {
    imageCount = total
    emitSnapshot()
  })

  loader
    .loadSeries(viewer, {
      studyUid: params.studyUid,
      seriesUid: params.seriesUid,
    })
    .then(() => {
      if (!destroyed) {
        viewer.render()
        emitSnapshot()
      }
    })
    .catch((error: unknown) => {
      if (!destroyed && !(error instanceof DOMException && error.name === 'AbortError')) {
        console.error('[dicomview] Stack load failed:', error)
      }
    })

  return {
    destroy() {
      if (destroyed) return
      destroyed = true
      listeners.clear()
      loader.abort()
      unbindInputs()
      viewer.destroy()
    },
    getSnapshot,
    reset() {
      viewer.reset()
      viewer.render()
      emitSnapshot()
    },
    setPrimaryTool(tool) {
      activeTool = tool
      emitSnapshot()
    },
    subscribe(listener) {
      listeners.add(listener)
      listener(getSnapshot())
      return () => {
        listeners.delete(listener)
      }
    },
  }
}
