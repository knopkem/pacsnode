import clsx from 'clsx'

interface ToggleProps {
  checked: boolean
  onCheckedChange: (checked: boolean) => void
  label: string
  description?: string
  compact?: boolean
}

export function Toggle({ checked, onCheckedChange, label, description, compact = false }: ToggleProps) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      onClick={() => onCheckedChange(!checked)}
      className={clsx(
        'flex w-full items-center justify-between gap-4 rounded-xl border border-slate-700 bg-slate-950/60 text-left transition hover:border-slate-600 hover:bg-slate-900/80',
        compact ? 'p-3' : 'p-4',
      )}
    >
      <span className="space-y-1">
        <span className="block text-sm font-semibold text-white">{label}</span>
        {description ? <span className="block text-sm text-slate-400">{description}</span> : null}
      </span>
      <span
        className={clsx(
          'relative h-7 w-12 rounded-full transition',
          checked ? 'bg-sky-500' : 'bg-slate-700',
        )}
      >
        <span
          className={clsx(
            'absolute top-1 h-5 w-5 rounded-full bg-white transition',
            checked ? 'left-6' : 'left-1',
          )}
        />
      </span>
    </button>
  )
}
