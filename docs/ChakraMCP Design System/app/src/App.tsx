import { Navigate, Route, Routes } from 'react-router-dom'
import './App.css'
import { ConceptPage } from './pages/ConceptPage'
import { DeveloperPage } from './pages/DeveloperPage'
import { PortfolioPage } from './pages/PortfolioPage'

function App() {
  return (
    <Routes>
      <Route path="/" element={<PortfolioPage />} />
      <Route path="/concept" element={<ConceptPage />} />
      <Route path="/developer" element={<DeveloperPage />} />
      <Route path="*" element={<Navigate to="/" replace />} />
    </Routes>
  )
}

export default App
