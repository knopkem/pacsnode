import clsx from 'clsx'
import { RotateCcw } from 'lucide-react'

import { isAnnotationTool } from '../../lib/engines/types'

export interface ToolDefinition {
  label: string
  tool: string
}

interface ViewportToolbarProps {
  tools: readonly ToolDefinition[]
  activeTool: string
  onToolChange: (tool: string) => void
  onReset: () => void
  ready: boolean
  /** When true, annotation tools are dimmed with a tooltip. */
  annotationsDisabled?: boolean
  /** Optional volume preset controls. */
  presetOptions?: readonly string[]
  selectedPreset?: string
  onPresetChange?: (preset: string) => void
  /** Optional right-side content (e.g. image counter). */
  rightContent?: React.ReactNode
}

export function ViewportToolbar({
  tools,
  activeTool,
  onToolChange,
  onReset,
  ready,
  annotationsDisabled = false,
  presetOptions,
  selectedPreset,
  onPresetChange,
  rightContent,
}: ViewportToolbarProps) {
  return (
    <div className="absolute inset-x-0 top-0 z-10 flex items-center gap-1 bg-black/60 px-2 py-1 backdrop-blur-sm">
      {tools.map(({ label, tool }) => {
        const isDisabled = !ready || (annotationsDisabled && isAnnotationTool(tool))
        return (
          <button
            key={tool}
            type="button"
            disabled={isDisabled}
            onClick={() => onToolChange(tool)}
            title={
              annotationsDisabled && isAnnotationTool(tool)
                ? 'Not available with dicomview engine'
                : undefined
            }
            className={clsx(
              'toolbar-button',
              activeTool === tool ? 'toolbar-button-active' : 'toolbar-button-inactive',
              isDisabled && 'cursor-not-allowed opacity-40',
            )}
          >
            {label}
          </button>
        )
      })}

      {presetOptions && presetOptions.length > 0 ? (
        <select
          value={selectedPreset ?? ''}
          disabled={!ready}
          onChange={(e) => onPresetChange?.(e.target.value)}
          className={clsx(
            'ml-auto h-7 rounded border border-slate-700 bg-slate-900 px-1.5 text-xs text-slate-200',
            !ready && 'opacity-40',
          )}
        >
          {presetOptions.map((p) => (
            <option key={p} value={p}>
              {p}
            </option>
          ))}
        </select>
      ) : null}

      <div className={clsx('flex items-center gap-2', !presetOptions?.length && 'ml-auto')}>
        {rightContent}
        <button
          type="button"
          disabled={!ready}
          onClick={onReset}
          className={clsx(
            'toolbar-button toolbar-button-inactive',
            !ready && 'cursor-not-allowed opacity-40',
          )}
        >
          <RotateCcw className="h-3 w-3" />
        </button>
      </div>
    </div>
  )
}
