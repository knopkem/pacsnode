import clsx from 'clsx'
import { ChevronLeft, Settings } from 'lucide-react'
import { NavLink, Outlet, useLocation, useNavigate, useParams } from 'react-router-dom'

const navItems = [
  { to: '/studies', label: 'Studies' },
]

export function AppShell() {
  const location = useLocation()
  const navigate = useNavigate()
  const params = useParams<{ studyUid: string }>()
  const isViewer = location.pathname.startsWith('/viewer/')
  const isSettings = location.pathname === '/settings'

  return (
    <div className="flex h-full flex-col">
      <header className="flex h-10 shrink-0 items-center gap-3 border-b border-slate-800 bg-slate-900/90 px-3">
        {isViewer ? (
          <NavLink
            to="/studies"
            className="flex items-center gap-1 text-xs text-slate-400 hover:text-white"
          >
            <ChevronLeft className="h-3.5 w-3.5" />
            Studies
          </NavLink>
        ) : isSettings ? (
          <button
            type="button"
            onClick={() => navigate(-1)}
            className="flex items-center gap-1 text-xs text-slate-400 hover:text-white"
          >
            <ChevronLeft className="h-3.5 w-3.5" />
            Back
          </button>
        ) : (
          <>
            <span className="text-sm font-semibold text-white">Pacsleaf</span>
            <nav className="flex gap-1">
              {navItems.map(({ to, label }) => (
                <NavLink
                  key={to}
                  to={to}
                  className={({ isActive }) =>
                    clsx(
                      'rounded px-2 py-1 text-xs font-medium transition',
                      isActive
                        ? 'bg-slate-800 text-white'
                        : 'text-slate-400 hover:bg-slate-800/60 hover:text-white',
                    )
                  }
                >
                  {label}
                </NavLink>
              ))}
            </nav>
          </>
        )}

        <div className="ml-auto flex items-center gap-2">
          {isViewer && params.studyUid ? (
            <span className="text-xs text-slate-500 truncate max-w-xs">
              {params.studyUid}
            </span>
          ) : null}
          <button
            type="button"
            onClick={() => isSettings ? navigate(-1) : navigate('/settings')}
            className={clsx(
              'rounded p-1.5 transition',
              isSettings
                ? 'bg-slate-800 text-white'
                : 'text-slate-500 hover:bg-slate-800/60 hover:text-white',
            )}
          >
            <Settings className="h-3.5 w-3.5" />
          </button>
        </div>
      </header>

      <main className="flex-1 overflow-hidden">
        <Outlet />
      </main>
    </div>
  )
}
