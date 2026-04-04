export interface DicomAttribute {
  vr: string
  Value?: unknown[]
  BulkDataURI?: string
  InlineBinary?: string
}

export type DicomJson = Record<string, DicomAttribute>

export interface StudySearchParams {
  patientName?: string
  patientId?: string
  accessionNumber?: string
  studyUid?: string
  modality?: string
  studyDateFrom?: string
  studyDateTo?: string
  limit?: number
  offset?: number
  fuzzyMatching?: boolean
}

export interface StudySummary {
  studyUid: string
  patientName?: string
  patientId?: string
  studyDate?: string
  accessionNumber?: string
  modalities: string[]
  description?: string
  referringPhysician?: string
  numSeries?: number
  numInstances?: number
  raw: DicomJson
}

export interface SeriesSummary {
  studyUid: string
  seriesUid: string
  modality?: string
  seriesNumber?: number
  description?: string
  bodyPart?: string
  numInstances?: number
  raw: DicomJson
}

export interface InstanceSummary {
  studyUid: string
  seriesUid: string
  instanceUid: string
  sopClassUid?: string
  instanceNumber?: number
  rows?: number
  columns?: number
  numberOfFrames?: number
  imageType: string[]
  photometricInterpretation?: string
  raw: DicomJson
}
