import type { InstanceSummary } from './dicomweb/types'

const countFormatter = new Intl.NumberFormat()

export function formatDicomDate(value?: string): string {
  if (!value) {
    return '—'
  }

  if (/^\d{8}$/.test(value)) {
    const year = Number.parseInt(value.slice(0, 4), 10)
    const month = Number.parseInt(value.slice(4, 6), 10) - 1
    const day = Number.parseInt(value.slice(6, 8), 10)
    const date = new Date(year, month, day)

    if (!Number.isNaN(date.getTime())) {
      return new Intl.DateTimeFormat(undefined, {
        year: 'numeric',
        month: 'short',
        day: 'numeric',
      }).format(date)
    }
  }

  return value
}

export function formatCount(value?: number): string {
  return typeof value === 'number' ? countFormatter.format(value) : '—'
}

export function formatModalities(modalities: string[]): string {
  return modalities.length > 0 ? modalities.join(' · ') : 'Unspecified'
}

export function formatValue(value?: string | number | null): string {
  if (value === undefined || value === null || value === '') {
    return '—'
  }

  return String(value)
}

export function formatInstanceResolution(instance?: InstanceSummary): string {
  if (!instance?.rows || !instance.columns) {
    return 'Unknown'
  }

  const frames = instance.numberOfFrames && instance.numberOfFrames > 1
    ? ` · ${countFormatter.format(instance.numberOfFrames)} frames`
    : ''

  return `${countFormatter.format(instance.columns)} × ${countFormatter.format(instance.rows)}${frames}`
}

export function truncateUid(uid?: string): string {
  if (!uid) {
    return '—'
  }

  return uid.length > 28 ? `${uid.slice(0, 14)}…${uid.slice(-10)}` : uid
}
