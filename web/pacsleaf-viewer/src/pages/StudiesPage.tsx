import clsx from 'clsx'
import { ChevronRight, Loader2, Search } from 'lucide-react'
import type { FormEvent } from 'react'
import { useMemo, useState } from 'react'
import { useNavigate } from 'react-router-dom'

import { useStudiesQuery } from '../lib/dicomweb/query-hooks'
import type { StudySearchParams, StudySummary } from '../lib/dicomweb/types'
import { formatCount, formatDicomDate, formatModalities } from '../lib/format'
import { useViewerPreferencesStore } from '../state/preferences'

const modalityOptions = ['', 'CT', 'MR', 'US', 'XA', 'MG', 'CR', 'DX', 'PT', 'NM']

interface StudySearchFormState {
  patientName: string
  patientId: string
  accessionNumber: string
  modality: string
  studyDateFrom: string
  studyDateTo: string
  limit: string
  fuzzyMatching: boolean
}

function createInitialForm(defaultLimit: number): StudySearchFormState {
  return {
    patientName: '',
    patientId: '',
    accessionNumber: '',
    modality: '',
    studyDateFrom: '',
    studyDateTo: '',
    limit: String(defaultLimit),
    fuzzyMatching: true,
  }
}

function toStudySearchParams(form: StudySearchFormState, defaultLimit: number): StudySearchParams {
  const parsedLimit = Number.parseInt(form.limit, 10)

  return {
    patientName: form.patientName.trim() || undefined,
    patientId: form.patientId.trim() || undefined,
    accessionNumber: form.accessionNumber.trim() || undefined,
    modality: form.modality || undefined,
    studyDateFrom: form.studyDateFrom || undefined,
    studyDateTo: form.studyDateTo || undefined,
    limit: Number.isFinite(parsedLimit) ? parsedLimit : defaultLimit,
    fuzzyMatching: form.fuzzyMatching,
  }
}

function StudyRow({ study, onClick }: { study: StudySummary; onClick: () => void }) {
  return (
    <tr
      onClick={onClick}
      className="cursor-pointer border-b border-slate-800/60 transition hover:bg-slate-800/40"
    >
      <td className="px-3 py-2.5 text-sm font-medium text-white">
        {study.patientName ?? '—'}
      </td>
      <td className="px-3 py-2.5 text-sm text-slate-300">
        {study.patientId ?? '—'}
      </td>
      <td className="px-3 py-2.5 text-sm text-slate-300 whitespace-nowrap">
        {formatDicomDate(study.studyDate)}
      </td>
      <td className="px-3 py-2.5 text-sm text-slate-300 max-w-xs truncate">
        {study.description ?? '—'}
      </td>
      <td className="px-3 py-2.5 text-sm">
        {study.modalities.length > 0 ? (
          <span className="text-slate-200">{formatModalities(study.modalities)}</span>
        ) : (
          <span className="text-slate-500">—</span>
        )}
      </td>
      <td className="px-3 py-2.5 text-sm text-slate-400">
        {study.accessionNumber ?? '—'}
      </td>
      <td className="px-3 py-2.5 text-sm text-slate-400 text-right tabular-nums">
        {formatCount(study.numInstances)}
      </td>
      <td className="px-3 py-2.5 text-right">
        <ChevronRight className="inline h-4 w-4 text-slate-600" />
      </td>
    </tr>
  )
}

export function StudiesPage() {
  const navigate = useNavigate()
  const defaultStudyLimit = useViewerPreferencesStore((state) => state.defaultStudyLimit)

  const [formState, setFormState] = useState<StudySearchFormState>(() =>
    createInitialForm(defaultStudyLimit),
  )
  const [submittedForm, setSubmittedForm] = useState<StudySearchFormState>(() =>
    createInitialForm(defaultStudyLimit),
  )

  const queryParams = useMemo(
    () => toStudySearchParams(submittedForm, defaultStudyLimit),
    [defaultStudyLimit, submittedForm],
  )
  const studiesQuery = useStudiesQuery(queryParams)
  const studies = studiesQuery.data ?? []

  function updateField<Key extends keyof StudySearchFormState>(key: Key, value: StudySearchFormState[Key]) {
    setFormState((current) => ({
      ...current,
      [key]: value,
    }))
  }

  function handleSearchSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    setSubmittedForm(formState)
  }

  function openStudy(study: StudySummary) {
    navigate(`/viewer/${encodeURIComponent(study.studyUid)}`, { state: { study } })
  }

  return (
    <div className="flex h-full flex-col overflow-hidden">
      {/* Header row with title + count */}
      <div className="flex items-center justify-between border-b border-slate-800 px-4 py-2.5">
        <h1 className="text-lg font-semibold text-white">Study List</h1>
        <span className="text-sm text-slate-400">
          {studiesQuery.isFetching ? (
            <Loader2 className="inline h-3.5 w-3.5 animate-spin" />
          ) : (
            <span>
              <span className="font-semibold text-sky-400">{studies.length}</span> Studies
            </span>
          )}
        </span>
      </div>

      {/* Table */}
      <div className="flex-1 overflow-auto scrollbar-thin">
        <form onSubmit={handleSearchSubmit}>
          <table className="w-full border-collapse text-left">
            <thead className="sticky top-0 z-10 bg-slate-900">
              <tr className="border-b border-slate-700">
                <th className="px-3 py-2">
                  <div className="text-xs font-medium text-slate-400 mb-1">Patient Name</div>
                  <input
                    className="field"
                    placeholder="Doe*"
                    value={formState.patientName}
                    onChange={(e) => updateField('patientName', e.target.value)}
                  />
                </th>
                <th className="px-3 py-2">
                  <div className="text-xs font-medium text-slate-400 mb-1">MRN</div>
                  <input
                    className="field"
                    placeholder=""
                    value={formState.patientId}
                    onChange={(e) => updateField('patientId', e.target.value)}
                  />
                </th>
                <th className="px-3 py-2">
                  <div className="text-xs font-medium text-slate-400 mb-1">Study Date</div>
                  <div className="flex gap-1">
                    <input
                      type="date"
                      className="field w-28"
                      value={formState.studyDateFrom}
                      onChange={(e) => updateField('studyDateFrom', e.target.value)}
                    />
                    <input
                      type="date"
                      className="field w-28"
                      value={formState.studyDateTo}
                      onChange={(e) => updateField('studyDateTo', e.target.value)}
                    />
                  </div>
                </th>
                <th className="px-3 py-2">
                  <div className="text-xs font-medium text-slate-400 mb-1">Description</div>
                  <input
                    className="field"
                    placeholder=""
                    value={formState.accessionNumber}
                    onChange={(e) => updateField('accessionNumber', e.target.value)}
                  />
                </th>
                <th className="px-3 py-2">
                  <div className="text-xs font-medium text-slate-400 mb-1">Modality</div>
                  <select
                    className="select-field"
                    value={formState.modality}
                    onChange={(e) => updateField('modality', e.target.value)}
                  >
                    <option value="">All</option>
                    {modalityOptions
                      .filter((o) => o)
                      .map((o) => (
                        <option key={o} value={o}>
                          {o}
                        </option>
                      ))}
                  </select>
                </th>
                <th className="px-3 py-2">
                  <div className="text-xs font-medium text-slate-400 mb-1">Accession</div>
                  <div className="h-8" />
                </th>
                <th className="px-3 py-2 text-right">
                  <div className="text-xs font-medium text-slate-400 mb-1">Instances</div>
                  <button
                    type="submit"
                    disabled={studiesQuery.isFetching}
                    className={clsx(
                      'inline-flex items-center gap-1 rounded bg-sky-600 px-2.5 py-1.5 text-xs font-medium text-white transition hover:bg-sky-500',
                      studiesQuery.isFetching && 'opacity-60',
                    )}
                  >
                    <Search className="h-3 w-3" />
                    Search
                  </button>
                </th>
                <th className="w-8" />
              </tr>
            </thead>
            <tbody>
              {studiesQuery.isPending ? (
                <tr>
                  <td colSpan={8} className="px-3 py-16 text-center">
                    <Loader2 className="mx-auto h-6 w-6 animate-spin text-slate-500" />
                    <p className="mt-2 text-sm text-slate-500">Loading studies…</p>
                  </td>
                </tr>
              ) : studiesQuery.isError ? (
                <tr>
                  <td colSpan={8} className="px-3 py-16 text-center text-sm text-rose-400">
                    {studiesQuery.error.message}
                  </td>
                </tr>
              ) : studies.length === 0 ? (
                <tr>
                  <td colSpan={8} className="px-3 py-16 text-center">
                    <Search className="mx-auto h-8 w-8 text-slate-600" />
                    <p className="mt-2 text-sm text-slate-500">No studies found</p>
                  </td>
                </tr>
              ) : (
                studies.map((study) => (
                  <StudyRow
                    key={study.studyUid}
                    study={study}
                    onClick={() => openStudy(study)}
                  />
                ))
              )}
            </tbody>
          </table>
        </form>
      </div>
    </div>
  )
}
