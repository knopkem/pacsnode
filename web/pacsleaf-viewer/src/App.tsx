import { Navigate, Route, Routes } from 'react-router-dom'

import { AppShell } from './components/layout/AppShell'
import { SettingsPage } from './pages/SettingsPage'
import { StudiesPage } from './pages/StudiesPage'
import { ViewerPage } from './pages/ViewerPage'

function App() {
  return (
    <Routes>
      <Route element={<AppShell />}>
        <Route index element={<Navigate replace to="/studies" />} />
        <Route path="/studies" element={<StudiesPage />} />
        <Route path="/viewer/:studyUid" element={<ViewerPage />} />
        <Route path="/settings" element={<SettingsPage />} />
      </Route>
      <Route path="*" element={<Navigate replace to="/studies" />} />
    </Routes>
  )
}

export default App
