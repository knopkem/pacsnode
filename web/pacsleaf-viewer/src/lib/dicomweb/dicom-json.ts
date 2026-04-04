import type { DicomJson, InstanceSummary, SeriesSummary, StudySummary } from './types'

const DICOM_TAGS = {
  accessionNumber: '00080050',
  bodyPartExamined: '00180015',
  columns: '00280011',
  imageType: '00080008',
  instanceNumber: '00200013',
  modalitiesInStudy: '00080061',
  modality: '00080060',
  numberOfFrames: '00280008',
  numberOfSeriesRelatedInstances: '00201209',
  numberOfStudyRelatedInstances: '00201208',
  numberOfStudyRelatedSeries: '00201206',
  patientId: '00100020',
  patientName: '00100010',
  photometricInterpretation: '00280004',
  referringPhysicianName: '00080090',
  rows: '00280010',
  seriesDescription: '0008103E',
  seriesInstanceUid: '0020000E',
  seriesNumber: '00200011',
  sopClassUid: '00080016',
  sopInstanceUid: '00080018',
  studyDate: '00080020',
  studyDescription: '00081030',
  studyInstanceUid: '0020000D',
} as const

type PersonNameValue = {
  Alphabetic?: string
  Ideographic?: string
  Phonetic?: string
}

function getFirstValue(dataset: DicomJson, tag: string): unknown {
  return dataset[tag]?.Value?.[0]
}

function toStringValue(value: unknown): string | undefined {
  if (typeof value === 'string') {
    return value
  }

  if (typeof value === 'number') {
    return String(value)
  }

  return undefined
}

function toDecimalValue(value: unknown): number | undefined {
  if (typeof value === 'number' && Number.isFinite(value)) {
    return value
  }

  if (typeof value === 'string') {
    const parsed = Number.parseFloat(value)
    return Number.isFinite(parsed) ? parsed : undefined
  }

  return undefined
}

export function getString(dataset: DicomJson, tag: string): string | undefined {
  return toStringValue(getFirstValue(dataset, tag))
}

export function getStringList(dataset: DicomJson, tag: string): string[] {
  return (dataset[tag]?.Value ?? [])
    .map((value) => toStringValue(value))
    .filter((value): value is string => Boolean(value))
}

export function getDecimalList(dataset: DicomJson, tag: string): number[] {
  return (dataset[tag]?.Value ?? [])
    .map((value) => toDecimalValue(value))
    .filter((value): value is number => value !== undefined)
}

export function getPersonName(dataset: DicomJson, tag: string): string | undefined {
  const value = getFirstValue(dataset, tag)

  if (typeof value === 'string') {
    return value
  }

  if (value && typeof value === 'object') {
    const personName = value as PersonNameValue
    return personName.Alphabetic ?? personName.Ideographic ?? personName.Phonetic
  }

  return undefined
}

export function getInteger(dataset: DicomJson, tag: string): number | undefined {
  const value = getFirstValue(dataset, tag)

  if (typeof value === 'number' && Number.isFinite(value)) {
    return value
  }

  if (typeof value === 'string') {
    const parsed = Number.parseInt(value, 10)
    return Number.isFinite(parsed) ? parsed : undefined
  }

  return undefined
}

export function parseStudySummary(dataset: DicomJson): StudySummary {
  return {
    studyUid: getString(dataset, DICOM_TAGS.studyInstanceUid) ?? 'unknown-study',
    patientName: getPersonName(dataset, DICOM_TAGS.patientName),
    patientId: getString(dataset, DICOM_TAGS.patientId),
    studyDate: getString(dataset, DICOM_TAGS.studyDate),
    accessionNumber: getString(dataset, DICOM_TAGS.accessionNumber),
    modalities: getStringList(dataset, DICOM_TAGS.modalitiesInStudy),
    description: getString(dataset, DICOM_TAGS.studyDescription),
    referringPhysician: getPersonName(dataset, DICOM_TAGS.referringPhysicianName),
    numSeries: getInteger(dataset, DICOM_TAGS.numberOfStudyRelatedSeries),
    numInstances: getInteger(dataset, DICOM_TAGS.numberOfStudyRelatedInstances),
    raw: dataset,
  }
}

export function parseSeriesSummary(dataset: DicomJson): SeriesSummary {
  return {
    studyUid: getString(dataset, DICOM_TAGS.studyInstanceUid) ?? 'unknown-study',
    seriesUid: getString(dataset, DICOM_TAGS.seriesInstanceUid) ?? 'unknown-series',
    modality: getString(dataset, DICOM_TAGS.modality),
    seriesNumber: getInteger(dataset, DICOM_TAGS.seriesNumber),
    description: getString(dataset, DICOM_TAGS.seriesDescription),
    bodyPart: getString(dataset, DICOM_TAGS.bodyPartExamined),
    numInstances: getInteger(dataset, DICOM_TAGS.numberOfSeriesRelatedInstances),
    raw: dataset,
  }
}

export function parseInstanceSummary(dataset: DicomJson): InstanceSummary {
  return {
    studyUid: getString(dataset, DICOM_TAGS.studyInstanceUid) ?? 'unknown-study',
    seriesUid: getString(dataset, DICOM_TAGS.seriesInstanceUid) ?? 'unknown-series',
    instanceUid: getString(dataset, DICOM_TAGS.sopInstanceUid) ?? 'unknown-instance',
    sopClassUid: getString(dataset, DICOM_TAGS.sopClassUid),
    instanceNumber: getInteger(dataset, DICOM_TAGS.instanceNumber),
    rows: getInteger(dataset, DICOM_TAGS.rows),
    columns: getInteger(dataset, DICOM_TAGS.columns),
    numberOfFrames: getInteger(dataset, DICOM_TAGS.numberOfFrames),
    imageType: getStringList(dataset, DICOM_TAGS.imageType),
    photometricInterpretation: getString(dataset, DICOM_TAGS.photometricInterpretation),
    raw: dataset,
  }
}
