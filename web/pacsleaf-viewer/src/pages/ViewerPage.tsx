import clsx from 'clsx'
import {
  AlertTriangle,
  LayoutGrid,
  LayoutPanelTop,
  Loader2,
} from 'lucide-react'
import { lazy, Suspense, useCallback, useMemo, useState } from 'react'
import { useLocation, useParams } from 'react-router-dom'

import { useStudyQuery, useStudySeriesQuery } from '../lib/dicomweb/query-hooks'
import type { StudySummary } from '../lib/dicomweb/types'
import { isEngineFallback, resolveEngine } from '../lib/engines/detect'
import { formatCount, formatDicomDate } from '../lib/format'
import { resolveStreamerUrl, useViewerPreferencesStore } from '../state/preferences'

type ViewerLocationState = {
  study?: StudySummary
}

type StreamingFallbackState = {
  key: string
  message: string
}

const StackViewport = lazy(async () => {
  const module = await import('../components/viewer/StackViewport')
  return { default: module.StackViewport }
})

const VolumeViewportGrid = lazy(async () => {
  const module = await import('../components/viewer/VolumeViewportGrid')
  return { default: module.VolumeViewportGrid }
})

const StreamingViewport = lazy(async () => {
  const module = await import('../components/viewer/StreamingViewport')
  return { default: module.StreamingViewport }
})

const DicomviewVolumeViewportGrid = lazy(async () => {
  const module = await import('../components/viewer/DicomviewVolumeViewportGrid')
  return { default: module.DicomviewVolumeViewportGrid }
})

const DicomviewStackViewport = lazy(async () => {
  const module = await import('../components/viewer/DicomviewStackViewport')
  return { default: module.DicomviewStackViewport }
})

export function ViewerPage() {
  const { studyUid } = useParams<{ studyUid: string }>()
  const location = useLocation()
  const locationState = location.state as ViewerLocationState | null
  const initialStudy =
    locationState?.study && locationState.study.studyUid === studyUid
      ? locationState.study
      : undefined

  const autoSelectFirstSeries = useViewerPreferencesStore((state) => state.autoSelectFirstSeries)
  const preferredRenderingMode = useViewerPreferencesStore(
    (state) => state.preferredRenderingMode,
  )
  const streamingQuality = useViewerPreferencesStore((state) => state.streamingQuality)
  const viewportLayout = useViewerPreferencesStore((state) => state.viewportLayout)
  const setViewportLayout = useViewerPreferencesStore((state) => state.setViewportLayout)
  const clientEngine = useViewerPreferencesStore((state) => state.clientEngine)

  const streamerUrl = useMemo(() => resolveStreamerUrl(), [])

  const resolvedEngine = useMemo(
    () => resolveEngine(clientEngine, viewportLayout),
    [clientEngine, viewportLayout],
  )
  const engineFallback = isEngineFallback(clientEngine, resolvedEngine)

  const studyQuery = useStudyQuery(studyUid, initialStudy)
  const seriesQuery = useStudySeriesQuery(studyUid)
  const [manualSelectedSeriesUid, setManualSelectedSeriesUid] = useState<string | undefined>(undefined)
  const [streamingFallback, setStreamingFallback] = useState<StreamingFallbackState | undefined>()

  const selectedSeriesUid = useMemo(() => {
    const series = seriesQuery.data ?? []
    if (!series.length) return undefined
    if (
      manualSelectedSeriesUid &&
      series.some((c) => c.seriesUid === manualSelectedSeriesUid)
    ) {
      return manualSelectedSeriesUid
    }
    return autoSelectFirstSeries ? series[0].seriesUid : undefined
  }, [autoSelectFirstSeries, manualSelectedSeriesUid, seriesQuery.data])

  const study = studyQuery.data ?? initialStudy
  const selectedSeries = useMemo(
    () => seriesQuery.data?.find((s) => s.seriesUid === selectedSeriesUid),
    [selectedSeriesUid, seriesQuery.data],
  )
  const streamingPreferred = preferredRenderingMode === 'streaming'
  const streamingContextKey = `${studyUid ?? ''}::${selectedSeriesUid ?? ''}::${streamerUrl}::${preferredRenderingMode}`
  const streamingFallbackMessage =
    streamingFallback?.key === streamingContextKey ? streamingFallback.message : undefined
  const effectiveRenderingMode =
    streamingPreferred && streamerUrl && !streamingFallbackMessage ? 'streaming' : 'client'
  const handleStreamingUnavailable = useCallback(
    (message: string) => {
      setStreamingFallback({ key: streamingContextKey, message })
    },
    [streamingContextKey],
  )

  if (!studyUid) {
    return (
      <div className="flex h-full items-center justify-center text-sm text-slate-400">
        No study selected. Return to the study list.
      </div>
    )
  }

  return (
    <div className="flex h-full overflow-hidden">
      {/* Series panel */}
      <aside className="flex w-32 shrink-0 flex-col border-r border-slate-800 bg-slate-900/80">
        <div className="border-b border-slate-800 px-2 py-2">
          <p className="text-[10px] font-medium text-slate-500 uppercase tracking-wider">Series</p>
        </div>
        <div className="flex-1 overflow-y-auto scrollbar-thin">
          {seriesQuery.isPending ? (
            <div className="flex items-center justify-center py-8">
              <Loader2 className="h-4 w-4 animate-spin text-slate-500" />
            </div>
          ) : seriesQuery.isError ? (
            <div className="p-2 text-xs text-rose-400">{seriesQuery.error.message}</div>
          ) : (
            (seriesQuery.data ?? []).map((series) => (
              <button
                type="button"
                key={series.seriesUid}
                onClick={() => setManualSelectedSeriesUid(series.seriesUid)}
                className={clsx(
                  'w-full border-b border-slate-800/50 px-2 py-2 text-left transition',
                  selectedSeriesUid === series.seriesUid
                    ? 'bg-sky-900/30 border-l-2 border-l-sky-400'
                    : 'hover:bg-slate-800/50',
                )}
              >
                <div className="flex items-center justify-between gap-1">
                  <span className="text-xs font-medium text-slate-200 truncate">
                    {series.description ?? `S${series.seriesNumber ?? '?'}`}
                  </span>
                  <span className="shrink-0 text-[10px] text-slate-500">
                    {series.modality ?? ''}
                  </span>
                </div>
                <div className="mt-0.5 text-[10px] text-slate-500">
                  {formatCount(series.numInstances)} img
                </div>
              </button>
            ))
          )}
        </div>
      </aside>

      {/* Main viewport area */}
      <div className="flex flex-1 flex-col overflow-hidden">
        {/* Patient info bar + layout toggles */}
        <div className="flex items-center justify-between border-b border-slate-800 bg-slate-900/60 px-3 py-1.5">
          <div className="flex items-center gap-3 text-xs">
            <span className="font-medium text-white">
              {study?.patientName ?? 'Loading…'}
            </span>
            <span className="text-slate-500">
              {formatDicomDate(study?.studyDate)}
            </span>
            {study?.description ? (
              <span className="text-slate-500 truncate max-w-xs">
                {study.description}
              </span>
            ) : null}
          </div>
          <div className="flex items-center gap-1">
            <button
              type="button"
              title="Single viewport"
              onClick={() => setViewportLayout('single')}
              className={clsx(
                'rounded p-1 transition',
                viewportLayout === 'single'
                  ? 'bg-slate-700 text-white'
                  : 'text-slate-500 hover:text-white',
              )}
            >
              <LayoutPanelTop className="h-3.5 w-3.5" />
            </button>
            <button
              type="button"
              title="Quad MPR + 3D"
              onClick={() => setViewportLayout('quad')}
              className={clsx(
                'rounded p-1 transition',
                viewportLayout === 'quad'
                  ? 'bg-slate-700 text-white'
                  : 'text-slate-500 hover:text-white',
              )}
            >
              <LayoutGrid className="h-3.5 w-3.5" />
            </button>
          </div>
        </div>

        {/* Streaming fallback banner */}
        {streamingFallbackMessage ? (
          <div className="flex items-center gap-2 border-b border-amber-800/40 bg-amber-900/20 px-3 py-1.5 text-xs text-amber-200">
            <AlertTriangle className="h-3 w-3 shrink-0" />
            <span>Streaming unavailable — using client rendering</span>
            {streamingPreferred && streamerUrl ? (
              <button
                type="button"
                onClick={() => setStreamingFallback(undefined)}
                className="ml-auto text-amber-300 underline hover:text-amber-100"
              >
                Retry
              </button>
            ) : null}
          </div>
        ) : null}

        {/* Engine fallback banner */}
        {engineFallback && effectiveRenderingMode === 'client' ? (
          <div className="flex items-center gap-2 border-b border-amber-800/40 bg-amber-900/20 px-3 py-1.5 text-xs text-amber-200">
            <AlertTriangle className="h-3 w-3 shrink-0" />
            <span>WebGPU not available — using cornerstone3D</span>
          </div>
        ) : null}

        {/* Viewport */}
        <div className="relative flex-1 overflow-hidden bg-black">
          {effectiveRenderingMode === 'client' && selectedSeries ? (
            <Suspense
              fallback={
                <div className="absolute inset-0 flex items-center justify-center text-sm text-slate-500">
                  <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                  Loading…
                </div>
              }
            >
              {resolvedEngine === 'dicomview' ? (
                viewportLayout === 'quad' ? (
                  <DicomviewVolumeViewportGrid
                    key={`${selectedSeries.seriesUid}-dv-quad`}
                    studyUid={studyUid}
                    seriesUid={selectedSeries.seriesUid}
                    modality={selectedSeries.modality}
                  />
                ) : (
                  <DicomviewStackViewport
                    key={`${selectedSeries.seriesUid}-dv-stack`}
                    studyUid={studyUid}
                    seriesUid={selectedSeries.seriesUid}
                  />
                )
              ) : viewportLayout === 'quad' ? (
                <VolumeViewportGrid
                  key={`${selectedSeries.seriesUid}-cs-quad`}
                  studyUid={studyUid}
                  seriesUid={selectedSeries.seriesUid}
                  modality={selectedSeries.modality}
                />
              ) : (
                <StackViewport
                  key={`${selectedSeries.seriesUid}-cs-stack`}
                  studyUid={studyUid}
                  seriesUid={selectedSeries.seriesUid}
                />
              )}
            </Suspense>
          ) : effectiveRenderingMode === 'streaming' && selectedSeries ? (
            <Suspense
              fallback={
                <div className="absolute inset-0 flex items-center justify-center text-sm text-slate-500">
                  <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                  Connecting…
                </div>
              }
            >
              <StreamingViewport
                key={`${selectedSeries.seriesUid}-streaming`}
                streamerUrl={streamerUrl}
                studyUid={studyUid}
                seriesUid={selectedSeries.seriesUid}
                quality={streamingQuality}
                layout={viewportLayout}
                onUnavailable={handleStreamingUnavailable}
              />
            </Suspense>
          ) : (
            <div className="absolute inset-0 flex flex-col items-center justify-center gap-2 text-slate-500">
              <LayoutGrid className="h-8 w-8 text-slate-700" />
              <p className="text-sm">Select a series</p>
            </div>
          )}
        </div>
      </div>
    </div>
  )
}
