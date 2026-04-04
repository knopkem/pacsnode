import { parseInstanceSummary, parseSeriesSummary, parseStudySummary } from './dicom-json'
import type { DicomJson, StudySearchParams, StudySummary } from './types'
import { getRuntimeConfig } from '../runtime-config'

function appendIfPresent(searchParams: URLSearchParams, key: string, value: number | string | undefined) {
  if (value === undefined || value === '') {
    return
  }

  searchParams.set(key, String(value))
}

function normalizeDateInput(value?: string): string | undefined {
  return value ? value.replaceAll('-', '') : undefined
}

function buildStudyDateQuery(from?: string, to?: string): string | undefined {
  const start = normalizeDateInput(from)
  const end = normalizeDateInput(to)

  if (start && end) {
    return `${start}-${end}`
  }

  if (start) {
    return start
  }

  if (end) {
    return `-${end}`
  }

  return undefined
}

async function fetchJson<T>(url: string, signal?: AbortSignal): Promise<T> {
  const response = await fetch(url, {
    signal,
    credentials: 'same-origin',
    headers: {
      Accept: 'application/json, application/dicom+json;q=0.9',
    },
  })

  if (!response.ok) {
    const message = (await response.text()).trim()
    throw new Error(message || `DICOMweb request failed with status ${response.status}`)
  }

  return response.json() as Promise<T>
}

function withQuery(path: string, searchParams: URLSearchParams): string {
  const queryString = searchParams.toString()
  return queryString ? `${path}?${queryString}` : path
}

export class DicomWebClient {
  readonly runtimeConfig = getRuntimeConfig()

  async searchStudies(params: StudySearchParams, signal?: AbortSignal) {
    const searchParams = new URLSearchParams()
    appendIfPresent(searchParams, 'PatientName', params.patientName)
    appendIfPresent(searchParams, 'PatientID', params.patientId)
    appendIfPresent(searchParams, 'AccessionNumber', params.accessionNumber)
    appendIfPresent(searchParams, 'StudyInstanceUID', params.studyUid)
    appendIfPresent(searchParams, 'Modality', params.modality)
    appendIfPresent(searchParams, 'limit', params.limit)
    appendIfPresent(searchParams, 'offset', params.offset)

    if (params.fuzzyMatching) {
      searchParams.set('fuzzymatching', 'true')
    }

    appendIfPresent(searchParams, 'StudyDate', buildStudyDateQuery(params.studyDateFrom, params.studyDateTo))

    const url = withQuery(`${this.runtimeConfig.dicomweb.qidoRoot}/studies`, searchParams)
    const datasets = await fetchJson<DicomJson[]>(url, signal)
    return datasets.map(parseStudySummary)
  }

  async getStudy(studyUid: string, signal?: AbortSignal): Promise<StudySummary> {
    const studies = await this.searchStudies({ studyUid, limit: 1 }, signal)
    const study = studies.find((candidate) => candidate.studyUid === studyUid)

    if (!study) {
      throw new Error(`Study ${studyUid} was not found in pacsnode.`)
    }

    return study
  }

  async searchSeries(studyUid: string, signal?: AbortSignal) {
    const datasets = await fetchJson<DicomJson[]>(
      `${this.runtimeConfig.dicomweb.qidoRoot}/studies/${encodeURIComponent(studyUid)}/series`,
      signal,
    )

    return datasets
      .map(parseSeriesSummary)
      .sort(
        (left, right) =>
          (left.seriesNumber ?? Number.MAX_SAFE_INTEGER) -
          (right.seriesNumber ?? Number.MAX_SAFE_INTEGER),
      )
  }

  async searchInstances(
    studyUid: string,
    seriesUid: string,
    limit?: number,
    signal?: AbortSignal,
  ) {
    const searchParams = new URLSearchParams()
    appendIfPresent(searchParams, 'limit', limit)

    const url = withQuery(
      `${this.runtimeConfig.dicomweb.qidoRoot}/studies/${encodeURIComponent(studyUid)}/series/${encodeURIComponent(seriesUid)}/instances`,
      searchParams,
    )

    const datasets = await fetchJson<DicomJson[]>(url, signal)

    return datasets
      .map(parseInstanceSummary)
      .sort(
        (left, right) =>
          (left.instanceNumber ?? Number.MAX_SAFE_INTEGER) -
          (right.instanceNumber ?? Number.MAX_SAFE_INTEGER),
      )
  }

  async getSeriesMetadata(
    studyUid: string,
    seriesUid: string,
    signal?: AbortSignal,
  ): Promise<DicomJson[]> {
    return fetchJson<DicomJson[]>(
      this.buildSeriesMetadataUrl(studyUid, seriesUid),
      signal,
    )
  }

  buildStudyMetadataUrl(studyUid: string): string {
    return `${this.runtimeConfig.dicomweb.wadoRoot}/studies/${encodeURIComponent(studyUid)}/metadata`
  }

  buildSeriesMetadataUrl(studyUid: string, seriesUid: string): string {
    return `${this.runtimeConfig.dicomweb.wadoRoot}/studies/${encodeURIComponent(studyUid)}/series/${encodeURIComponent(seriesUid)}/metadata`
  }

  buildInstanceUrl(studyUid: string, seriesUid: string, instanceUid: string): string {
    return `${this.runtimeConfig.dicomweb.wadoRoot}/studies/${encodeURIComponent(studyUid)}/series/${encodeURIComponent(seriesUid)}/instances/${encodeURIComponent(instanceUid)}`
  }

  buildInstanceMetadataUrl(studyUid: string, seriesUid: string, instanceUid: string): string {
    return `${this.runtimeConfig.dicomweb.wadoRoot}/studies/${encodeURIComponent(studyUid)}/series/${encodeURIComponent(seriesUid)}/instances/${encodeURIComponent(instanceUid)}/metadata`
  }

}

export const dicomWebClient = new DicomWebClient()
