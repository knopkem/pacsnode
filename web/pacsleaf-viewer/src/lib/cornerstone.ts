import cornerstoneDICOMImageLoader, {
  init as initDicomImageLoader,
} from '@cornerstonejs/dicom-image-loader'
import {
  Enums as CoreEnums,
  RenderingEngine,
  eventTarget,
  setVolumesForViewports,
  type Types,
  init as initCornerstone,
  volumeLoader,
} from '@cornerstonejs/core'
import {
  addTool,
  annotation,
  AngleTool,
  CrosshairsTool,
  EllipticalROITool,
  Enums as ToolsEnums,
  init as initCornerstoneTools,
  LengthTool,
  PanTool,
  RectangleROITool,
  StackScrollTool,
  ToolGroupManager,
  TrackballRotateTool,
  WindowLevelTool,
  ZoomTool,
} from '@cornerstonejs/tools'

import { dicomWebClient } from './dicomweb/client'
import { getDecimalList, getInteger, getString } from './dicomweb/dicom-json'
import type { DicomJson } from './dicomweb/types'

let initializationPromise: Promise<void> | undefined
const registeredToolNames = new Set<string>()

type WadorsMetadata = Parameters<
  typeof cornerstoneDICOMImageLoader.wadors.metaDataManager.add
>[1]
type Point3 = readonly [number, number, number]
type SnapshotListener<T> = (snapshot: T) => void

export type StackPrimaryTool =
  | 'windowLevel'
  | 'pan'
  | 'zoom'
  | 'length'
  | 'angle'
  | 'rectangleRoi'
  | 'ellipticalRoi'

export interface StackViewportSnapshot {
  activeTool: StackPrimaryTool
  currentImageIndex: number
  imageCount: number
  measurementCount: number
}

export interface StackViewportController {
  destroy(): void
  getSnapshot(): StackViewportSnapshot
  reset(): void
  setPrimaryTool(tool: StackPrimaryTool): void
  subscribe(listener: SnapshotListener<StackViewportSnapshot>): () => void
}

export type MprPrimaryTool =
  | 'crosshairs'
  | 'windowLevel'
  | 'pan'
  | 'zoom'
  | 'length'
  | 'angle'
  | 'rectangleRoi'
  | 'ellipticalRoi'

export interface VolumeViewportSnapshot {
  activeTool: MprPrimaryTool
  imageCount: number
  measurementCount: number
  volumePreset?: string
}

export interface VolumeViewportController {
  destroy(): void
  getSnapshot(): VolumeViewportSnapshot
  reset(): void
  setPrimaryTool(tool: MprPrimaryTool): void
  setVolumePreset(preset?: string): void
  subscribe(listener: SnapshotListener<VolumeViewportSnapshot>): () => void
}

export interface VolumeViewportElements {
  axial: HTMLDivElement
  coronal: HTMLDivElement
  sagittal: HTMLDivElement
  volume: HTMLDivElement
}

const DICOM_TAGS = {
  imageOrientationPatient: '00200037',
  imagePositionPatient: '00200032',
  instanceNumber: '00200013',
  numberOfFrames: '00280008',
  sopInstanceUid: '00080018',
} as const

const MEASUREMENT_TOOL_NAMES = new Set([
  AngleTool.toolName,
  EllipticalROITool.toolName,
  LengthTool.toolName,
  RectangleROITool.toolName,
])

const STACK_PRIMARY_TOOL_NAMES: Record<StackPrimaryTool, string> = {
  windowLevel: WindowLevelTool.toolName,
  pan: PanTool.toolName,
  zoom: ZoomTool.toolName,
  length: LengthTool.toolName,
  angle: AngleTool.toolName,
  rectangleRoi: RectangleROITool.toolName,
  ellipticalRoi: EllipticalROITool.toolName,
}

const MPR_PRIMARY_TOOL_NAMES: Record<MprPrimaryTool, string> = {
  crosshairs: CrosshairsTool.toolName,
  windowLevel: WindowLevelTool.toolName,
  pan: PanTool.toolName,
  zoom: ZoomTool.toolName,
  length: LengthTool.toolName,
  angle: AngleTool.toolName,
  rectangleRoi: RectangleROITool.toolName,
  ellipticalRoi: EllipticalROITool.toolName,
}

const ANNOTATION_EVENTS = [
  ToolsEnums.Events.ANNOTATION_ADDED,
  ToolsEnums.Events.ANNOTATION_COMPLETED,
  ToolsEnums.Events.ANNOTATION_MODIFIED,
  ToolsEnums.Events.ANNOTATION_REMOVED,
] as const

function registerTool(toolClass: Parameters<typeof addTool>[0] & { toolName: string }) {
  if (registeredToolNames.has(toolClass.toolName)) {
    return
  }

  addTool(toolClass)
  registeredToolNames.add(toolClass.toolName)
}

function ensureCornerstoneToolsReady() {
  initCornerstoneTools()
  registerTool(WindowLevelTool)
  registerTool(PanTool)
  registerTool(ZoomTool)
  registerTool(StackScrollTool)
  registerTool(LengthTool)
  registerTool(AngleTool)
  registerTool(RectangleROITool)
  registerTool(EllipticalROITool)
  registerTool(CrosshairsTool)
  registerTool(TrackballRotateTool)
}

function createToolGroup(toolGroupId: string) {
  const toolGroup = ToolGroupManager.createToolGroup(toolGroupId)
  if (!toolGroup) {
    throw new Error(`Unable to create Cornerstone tool group ${toolGroupId}.`)
  }

  return toolGroup
}

function setPrimaryToolBinding(
  toolGroup: ReturnType<typeof createToolGroup>,
  toolName: string,
  availableToolNames: readonly string[],
) {
  for (const candidate of availableToolNames) {
    toolGroup.setToolPassive(candidate, { removeAllBindings: true })
  }

  toolGroup.setToolActive(toolName, {
    bindings: [{ mouseButton: ToolsEnums.MouseBindings.Primary }],
  })
}

function addAnnotationListeners(listener: EventListener) {
  for (const eventName of ANNOTATION_EVENTS) {
    eventTarget.addEventListener(eventName, listener)
  }

  return () => {
    for (const eventName of ANNOTATION_EVENTS) {
      eventTarget.removeEventListener(eventName, listener)
    }
  }
}

function countMeasurementsForElements(elements: readonly HTMLDivElement[]): number {
  let measurementCount = 0

  for (const element of elements) {
    for (const toolName of MEASUREMENT_TOOL_NAMES) {
      measurementCount += annotation.state.getAnnotations(toolName, element).length
    }
  }

  return measurementCount
}

function removeAnnotationsForElements(
  toolNames: readonly string[],
  elements: readonly HTMLDivElement[],
) {
  for (const element of elements) {
    for (const toolName of toolNames) {
      annotation.state.removeAnnotations(toolName, element)
    }
  }
}

function absoluteUrl(path: string): string {
  return new URL(path, window.location.origin).toString()
}

function toPoint3(values: number[]): Point3 | undefined {
  if (values.length !== 3) {
    return undefined
  }

  return [values[0], values[1], values[2]]
}

function getSlicePosition(metadata: DicomJson): Point3 | undefined {
  return toPoint3(getDecimalList(metadata, DICOM_TAGS.imagePositionPatient))
}

function getSliceNormal(metadata: DicomJson): Point3 | undefined {
  const orientation = getDecimalList(metadata, DICOM_TAGS.imageOrientationPatient)
  if (orientation.length !== 6) {
    return undefined
  }

  const row: Point3 = [orientation[0], orientation[1], orientation[2]]
  const column: Point3 = [orientation[3], orientation[4], orientation[5]]
  return [
    row[1] * column[2] - row[2] * column[1],
    row[2] * column[0] - row[0] * column[2],
    row[0] * column[1] - row[1] * column[0],
  ]
}

function dotProduct(left: Point3, right: Point3): number {
  return left[0] * right[0] + left[1] * right[1] + left[2] * right[2]
}

function compareSeriesMetadata(left: DicomJson, right: DicomJson): number {
  const sliceNormal = getSliceNormal(left) ?? getSliceNormal(right)
  const leftPosition = getSlicePosition(left)
  const rightPosition = getSlicePosition(right)

  if (sliceNormal && leftPosition && rightPosition) {
    const leftDistance = dotProduct(leftPosition, sliceNormal)
    const rightDistance = dotProduct(rightPosition, sliceNormal)
    const delta = leftDistance - rightDistance

    if (Math.abs(delta) > 0.001) {
      return delta
    }
  }

  const instanceDelta =
    (getInteger(left, DICOM_TAGS.instanceNumber) ?? Number.MAX_SAFE_INTEGER) -
    (getInteger(right, DICOM_TAGS.instanceNumber) ?? Number.MAX_SAFE_INTEGER)

  if (instanceDelta !== 0) {
    return instanceDelta
  }

  return (getString(left, DICOM_TAGS.sopInstanceUid) ?? '').localeCompare(
    getString(right, DICOM_TAGS.sopInstanceUid) ?? '',
  )
}

function createInstanceImageIds(
  metadata: DicomJson,
  studyUid: string,
  seriesUid: string,
  wadoRoot: string,
): string[] {
  const sopInstanceUid = getString(metadata, DICOM_TAGS.sopInstanceUid)
  if (!sopInstanceUid) {
    return []
  }

  const frameCount = Math.max(getInteger(metadata, DICOM_TAGS.numberOfFrames) ?? 1, 1)
  const imageIds: string[] = []

  for (let frameNumber = 1; frameNumber <= frameCount; frameNumber += 1) {
    const imageId =
      `wadors:${wadoRoot}/studies/${encodeURIComponent(studyUid)}` +
      `/series/${encodeURIComponent(seriesUid)}` +
      `/instances/${encodeURIComponent(sopInstanceUid)}/frames/${frameNumber}`

    cornerstoneDICOMImageLoader.wadors.metaDataManager.add(
      imageId,
      metadata as WadorsMetadata,
    )
    imageIds.push(imageId)
  }

  return imageIds
}

export async function ensureCornerstoneReady(): Promise<void> {
  if (!initializationPromise) {
    initializationPromise = (async () => {
      await initCornerstone()
      initDicomImageLoader({
        maxWebWorkers: Math.max(Math.min(navigator.hardwareConcurrency || 2, 4), 1),
      })
      ensureCornerstoneToolsReady()
    })()
  }

  return initializationPromise
}

export async function buildSeriesImageIds(
  studyUid: string,
  seriesUid: string,
  signal?: AbortSignal,
): Promise<string[]> {
  await ensureCornerstoneReady()

  const metadata = await dicomWebClient.getSeriesMetadata(studyUid, seriesUid, signal)
  const wadoRoot = absoluteUrl(dicomWebClient.runtimeConfig.dicomweb.wadoRoot)

  return [...metadata]
    .sort(compareSeriesMetadata)
    .flatMap((dataset) => createInstanceImageIds(dataset, studyUid, seriesUid, wadoRoot))
}

export async function mountStackViewport(params: {
  element: HTMLDivElement
  studyUid: string
  seriesUid: string
  renderingEngineId: string
  viewportId: string
  signal?: AbortSignal
}): Promise<StackViewportController> {
  const imageIds = await buildSeriesImageIds(params.studyUid, params.seriesUid, params.signal)

  if (imageIds.length === 0) {
    throw new Error('No WADO-RS image frames were found for the selected series.')
  }

  const renderingEngine = new RenderingEngine(params.renderingEngineId)
  renderingEngine.enableElement({
    viewportId: params.viewportId,
    type: CoreEnums.ViewportType.STACK,
    element: params.element,
  })

  const viewport = renderingEngine.getViewport(params.viewportId) as Types.IStackViewport
  const toolGroupId = `${params.viewportId}-tools`
  const toolGroup = createToolGroup(toolGroupId)

  for (const toolName of new Set([
    ...Object.values(STACK_PRIMARY_TOOL_NAMES),
    StackScrollTool.toolName,
  ])) {
    toolGroup.addTool(toolName)
  }

  toolGroup.addViewport(params.viewportId, params.renderingEngineId)
  await viewport.setStack(imageIds, 0)
  toolGroup.setToolActive(StackScrollTool.toolName, {
    bindings: [{ mouseButton: ToolsEnums.MouseBindings.Wheel }],
  })

  let activeTool: StackPrimaryTool = 'windowLevel'
  const listeners = new Set<SnapshotListener<StackViewportSnapshot>>()
  let destroyed = false

  // Keep Cornerstone's internal canvas in sync with the container size.
  const resizeObserver = new ResizeObserver(() => {
    if (!destroyed) {
      renderingEngine.resize(true, false)
    }
  })
  resizeObserver.observe(params.element)

  const getSnapshot = (): StackViewportSnapshot => ({
    activeTool,
    currentImageIndex: Math.max(viewport.getCurrentImageIdIndex(), 0),
    imageCount: imageIds.length,
    measurementCount: countMeasurementsForElements([params.element]),
  })

  const emitSnapshot = () => {
    const snapshot = getSnapshot()
    for (const listener of listeners) {
      listener(snapshot)
    }
  }

  const syncViewport = () => {
    if (!destroyed) {
      emitSnapshot()
    }
  }

  const annotationCleanup = addAnnotationListeners(syncViewport)
  params.element.addEventListener(CoreEnums.Events.STACK_NEW_IMAGE, syncViewport)
  setPrimaryToolBinding(
    toolGroup,
    STACK_PRIMARY_TOOL_NAMES[activeTool],
    Object.values(STACK_PRIMARY_TOOL_NAMES),
  )
  viewport.render()

  return {
    destroy() {
      if (destroyed) {
        return
      }

      destroyed = true
      resizeObserver.disconnect()
      listeners.clear()
      params.element.removeEventListener(CoreEnums.Events.STACK_NEW_IMAGE, syncViewport)
      annotationCleanup()
      removeAnnotationsForElements([...MEASUREMENT_TOOL_NAMES], [params.element])
      ToolGroupManager.destroyToolGroup(toolGroupId)
      renderingEngine.destroy()
    },
    getSnapshot,
    reset() {
      viewport.resetProperties()
      viewport.resetCamera()
      viewport.render()
      emitSnapshot()
    },
    setPrimaryTool(tool) {
      activeTool = tool
      setPrimaryToolBinding(
        toolGroup,
        STACK_PRIMARY_TOOL_NAMES[tool],
        Object.values(STACK_PRIMARY_TOOL_NAMES),
      )
      viewport.render()
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

function volumeViewportIds(renderingEngineId: string) {
  return {
    axial: `${renderingEngineId}-axial`,
    coronal: `${renderingEngineId}-coronal`,
    sagittal: `${renderingEngineId}-sagittal`,
    volume: `${renderingEngineId}-volume`,
  }
}

function seriesVolumeId(studyUid: string, seriesUid: string): string {
  return `cornerstoneStreamingImageVolume:${studyUid}:${seriesUid}`
}

export async function mountVolumeViewportGrid(params: {
  elements: VolumeViewportElements
  studyUid: string
  seriesUid: string
  renderingEngineId: string
  signal?: AbortSignal
  volumePreset?: string
}): Promise<VolumeViewportController> {
  const imageIds = await buildSeriesImageIds(params.studyUid, params.seriesUid, params.signal)

  if (imageIds.length < 2) {
    throw new Error('The selected series does not contain enough ordered slices for MPR rendering.')
  }

  const viewportIds = volumeViewportIds(params.renderingEngineId)
  const renderingEngine = new RenderingEngine(params.renderingEngineId)
  renderingEngine.setViewports([
    {
      viewportId: viewportIds.axial,
      type: CoreEnums.ViewportType.ORTHOGRAPHIC,
      element: params.elements.axial,
      defaultOptions: {
        orientation: CoreEnums.OrientationAxis.AXIAL,
      },
    },
    {
      viewportId: viewportIds.coronal,
      type: CoreEnums.ViewportType.ORTHOGRAPHIC,
      element: params.elements.coronal,
      defaultOptions: {
        orientation: CoreEnums.OrientationAxis.CORONAL,
      },
    },
    {
      viewportId: viewportIds.sagittal,
      type: CoreEnums.ViewportType.ORTHOGRAPHIC,
      element: params.elements.sagittal,
      defaultOptions: {
        orientation: CoreEnums.OrientationAxis.SAGITTAL,
      },
    },
    {
      viewportId: viewportIds.volume,
      type: CoreEnums.ViewportType.VOLUME_3D,
      element: params.elements.volume,
    },
  ])

  const mprToolGroupId = `${params.renderingEngineId}-mpr-tools`
  const volumeToolGroupId = `${params.renderingEngineId}-volume-tools`
  const mprToolGroup = createToolGroup(mprToolGroupId)
  const volumeToolGroup = createToolGroup(volumeToolGroupId)

  for (const toolName of new Set(Object.values(MPR_PRIMARY_TOOL_NAMES))) {
    mprToolGroup.addTool(toolName)
  }

  for (const viewportId of [viewportIds.axial, viewportIds.coronal, viewportIds.sagittal]) {
    mprToolGroup.addViewport(viewportId, params.renderingEngineId)
  }

  volumeToolGroup.addTool(TrackballRotateTool.toolName)
  volumeToolGroup.addTool(PanTool.toolName)
  volumeToolGroup.addTool(ZoomTool.toolName)
  volumeToolGroup.addViewport(viewportIds.volume, params.renderingEngineId)

  const volumeId = seriesVolumeId(params.studyUid, params.seriesUid)
  const volume = await volumeLoader.createAndCacheVolume(volumeId, { imageIds })

  if (params.signal?.aborted) {
    renderingEngine.destroy()
    throw new DOMException('The volume request was aborted.', 'AbortError')
  }

  await setVolumesForViewports(renderingEngine, [{ volumeId }], Object.values(viewportIds))
  volume.load()

  let activeTool: MprPrimaryTool = 'crosshairs'
  let currentVolumePreset = params.volumePreset

  setPrimaryToolBinding(
    mprToolGroup,
    MPR_PRIMARY_TOOL_NAMES[activeTool],
    Object.values(MPR_PRIMARY_TOOL_NAMES),
  )
  volumeToolGroup.setToolActive(TrackballRotateTool.toolName, {
    bindings: [{ mouseButton: ToolsEnums.MouseBindings.Primary }],
  })
  volumeToolGroup.setToolActive(PanTool.toolName, {
    bindings: [{ mouseButton: ToolsEnums.MouseBindings.Secondary }],
  })
  volumeToolGroup.setToolActive(ZoomTool.toolName, {
    bindings: [{ mouseButton: ToolsEnums.MouseBindings.Wheel }],
  })

  const orthographicViewportIds = [
    viewportIds.axial,
    viewportIds.coronal,
    viewportIds.sagittal,
  ] as const
  const orthographicElements = [
    params.elements.axial,
    params.elements.coronal,
    params.elements.sagittal,
  ] as const
  const trackedElements = [...orthographicElements, params.elements.volume] as const
  const listeners = new Set<SnapshotListener<VolumeViewportSnapshot>>()
  let destroyed = false
  const volumeViewport = renderingEngine.getViewport(viewportIds.volume) as Types.IVolumeViewport

  // Keep Cornerstone's internal canvases in sync with the container sizes.
  const resizeObserver = new ResizeObserver(() => {
    if (!destroyed) {
      renderingEngine.resize(true, false)
    }
  })
  for (const el of trackedElements) {
    resizeObserver.observe(el)
  }

  const getSnapshot = (): VolumeViewportSnapshot => ({
    activeTool,
    imageCount: imageIds.length,
    measurementCount: countMeasurementsForElements(orthographicElements),
    volumePreset: currentVolumePreset,
  })

  const emitSnapshot = () => {
    const snapshot = getSnapshot()
    for (const listener of listeners) {
      listener(snapshot)
    }
  }

  const syncViewport = () => {
    if (!destroyed) {
      emitSnapshot()
    }
  }

  const annotationCleanup = addAnnotationListeners(syncViewport)

  if (currentVolumePreset) {
    volumeViewport.setProperties({
      preset: currentVolumePreset,
    })
  }

  renderingEngine.renderViewports(Object.values(viewportIds))

  return {
    destroy() {
      if (destroyed) {
        return
      }

      destroyed = true
      resizeObserver.disconnect()
      listeners.clear()
      annotationCleanup()
      removeAnnotationsForElements(
        [...MEASUREMENT_TOOL_NAMES, CrosshairsTool.toolName],
        trackedElements,
      )
      ToolGroupManager.destroyToolGroup(mprToolGroupId)
      ToolGroupManager.destroyToolGroup(volumeToolGroupId)
      renderingEngine.destroy()
    },
    getSnapshot,
    reset() {
      const crosshairsTool = mprToolGroup.getToolInstance(CrosshairsTool.toolName) as
        | { resetCrosshairs?: () => void }
        | undefined

      for (const viewportId of orthographicViewportIds) {
        const mprViewport = renderingEngine.getViewport(viewportId) as Types.IVolumeViewport
        mprViewport.resetProperties(volumeId)
        mprViewport.resetCamera()
      }

      volumeViewport.resetProperties(volumeId)
      if (currentVolumePreset) {
        volumeViewport.setProperties({ preset: currentVolumePreset }, volumeId)
      }
      volumeViewport.resetCamera()
      crosshairsTool?.resetCrosshairs?.()
      renderingEngine.renderViewports(Object.values(viewportIds))
      emitSnapshot()
    },
    setPrimaryTool(tool) {
      activeTool = tool
      setPrimaryToolBinding(
        mprToolGroup,
        MPR_PRIMARY_TOOL_NAMES[tool],
        Object.values(MPR_PRIMARY_TOOL_NAMES),
      )
      renderingEngine.renderViewports([...orthographicViewportIds])
      emitSnapshot()
    },
    setVolumePreset(preset) {
      currentVolumePreset = preset
      volumeViewport.resetProperties(volumeId)
      if (preset) {
        volumeViewport.setProperties({ preset }, volumeId)
      }
      volumeViewport.render()
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
