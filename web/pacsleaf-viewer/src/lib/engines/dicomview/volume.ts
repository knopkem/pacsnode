import { DICOMwebLoader, Viewer } from '@knopkem/dicomview'
import type { VolumePreset } from '@knopkem/dicomview'

import { getRuntimeConfig } from '../../runtime-config'
import type { MprPrimaryTool } from '../types'
import { ensureDicomviewReady } from './init'
import { bindDicomviewInputs, bindVolumeCanvasInputs } from './mouse-handler'
import { toDicomviewPreset } from './preset-map'

type SnapshotListener = (snapshot: DicomviewVolumeSnapshot) => void

export interface DicomviewVolumeSnapshot {
  activeTool: MprPrimaryTool
  imageCount: number
  loadedCount: number
  volumePreset?: string
}

export interface DicomviewVolumeViewportElements {
  axial: HTMLCanvasElement
  coronal: HTMLCanvasElement
  sagittal: HTMLCanvasElement
  volume: HTMLCanvasElement
}

export interface DicomviewVolumeController {
  destroy(): void
  getSnapshot(): DicomviewVolumeSnapshot
  reset(): void
  setPrimaryTool(tool: MprPrimaryTool): void
  setVolumePreset(preset?: string): void
  subscribe(listener: SnapshotListener): () => void
}

export async function mountDicomviewVolumeGrid(params: {
  elements: DicomviewVolumeViewportElements
  studyUid: string
  seriesUid: string
  volumePreset?: string
  signal?: AbortSignal
}): Promise<DicomviewVolumeController> {
  await ensureDicomviewReady()

  if (params.signal?.aborted) {
    throw new DOMException('The volume request was aborted.', 'AbortError')
  }

  const viewer = await Viewer.create({
    axial: params.elements.axial,
    coronal: params.elements.coronal,
    sagittal: params.elements.sagittal,
    volume: params.elements.volume,
  })

  if (params.signal?.aborted) {
    viewer.destroy()
    throw new DOMException('The volume request was aborted.', 'AbortError')
  }

  let activeTool: MprPrimaryTool = 'crosshairs'
  let currentVolumePreset = params.volumePreset
  let imageCount = 0
  let loadedCount = 0
  let destroyed = false
  const listeners = new Set<SnapshotListener>()

  // Apply initial preset
  if (currentVolumePreset) {
    viewer.setVolumePreset(toDicomviewPreset(currentVolumePreset) as VolumePreset)
  }

  // Wire up mouse events for each MPR canvas
  const mprCanvases: Array<{
    canvas: HTMLCanvasElement
    viewport: 'axial' | 'coronal' | 'sagittal'
  }> = [
    { canvas: params.elements.axial, viewport: 'axial' },
    { canvas: params.elements.coronal, viewport: 'coronal' },
    { canvas: params.elements.sagittal, viewport: 'sagittal' },
  ]

  const unbindFns = mprCanvases.map(({ canvas, viewport }) =>
    bindDicomviewInputs(
      canvas,
      () => (destroyed ? undefined : viewer),
      () => activeTool,
      { viewportIndex: viewport },
    ),
  )

  // Volume canvas gets orbit interaction
  unbindFns.push(
    bindVolumeCanvasInputs(
      params.elements.volume,
      () => (destroyed ? undefined : viewer),
    ),
  )

  const getSnapshot = (): DicomviewVolumeSnapshot => ({
    activeTool,
    imageCount,
    loadedCount,
    volumePreset: currentVolumePreset,
  })

  const emitSnapshot = () => {
    const snapshot = getSnapshot()
    for (const listener of listeners) {
      listener(snapshot)
    }
  }

  // Load the series via DICOMwebLoader
  const runtimeConfig = getRuntimeConfig()
  const wadoRoot = new URL(
    runtimeConfig.dicomweb.wadoRoot,
    window.location.origin,
  ).toString()

  const loader = new DICOMwebLoader({ wadoRoot })
  loader.onProgress((loaded, total) => {
    loadedCount = loaded
    imageCount = total
    emitSnapshot()
  })

  // Start loading (don't await — it runs progressively)
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
        console.error('[dicomview] Volume load failed:', error)
      }
    })

  return {
    destroy() {
      if (destroyed) return
      destroyed = true
      listeners.clear()
      loader.abort()
      for (const unbind of unbindFns) {
        unbind()
      }
      viewer.destroy()
    },
    getSnapshot,
    reset() {
      viewer.reset()
      if (currentVolumePreset) {
        viewer.setVolumePreset(toDicomviewPreset(currentVolumePreset) as VolumePreset)
      }
      viewer.render()
      emitSnapshot()
    },
    setPrimaryTool(tool) {
      activeTool = tool
      emitSnapshot()
    },
    setVolumePreset(preset) {
      currentVolumePreset = preset
      if (preset) {
        viewer.setVolumePreset(toDicomviewPreset(preset) as VolumePreset)
      }
      viewer.render()
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
