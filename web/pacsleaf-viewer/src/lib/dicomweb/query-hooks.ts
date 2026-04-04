import { keepPreviousData, useQuery } from '@tanstack/react-query'

import { dicomWebClient } from './client'
import type { StudySearchParams, StudySummary } from './types'

export function useStudiesQuery(params: StudySearchParams) {
  return useQuery({
    queryKey: ['studies', params],
    queryFn: ({ signal }) => dicomWebClient.searchStudies(params, signal),
    placeholderData: keepPreviousData,
  })
}

export function useStudyQuery(studyUid: string | undefined, initialData?: StudySummary) {
  return useQuery({
    queryKey: ['study', studyUid],
    queryFn: ({ signal }) => {
      if (!studyUid) {
        throw new Error('Study UID is required to load study details.')
      }

      return dicomWebClient.getStudy(studyUid, signal)
    },
    enabled: Boolean(studyUid),
    initialData,
  })
}

export function useStudySeriesQuery(studyUid: string | undefined) {
  return useQuery({
    queryKey: ['series', studyUid],
    queryFn: ({ signal }) => {
      if (!studyUid) {
        throw new Error('Study UID is required to query series.')
      }

      return dicomWebClient.searchSeries(studyUid, signal)
    },
    enabled: Boolean(studyUid),
  })
}

export function useSeriesInstancesQuery(studyUid: string | undefined, seriesUid: string | undefined) {
  return useQuery({
    queryKey: ['instances', studyUid, seriesUid],
    queryFn: ({ signal }) => {
      if (!studyUid || !seriesUid) {
        throw new Error('Study and series UIDs are required to query instances.')
      }

      return dicomWebClient.searchInstances(studyUid, seriesUid, undefined, signal)
    },
    enabled: Boolean(studyUid && seriesUid),
    placeholderData: keepPreviousData,
  })
}
