import { BrowserRouter, Routes, Route, Navigate, NavLink } from 'react-router-dom';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { Dashboard } from './pages/Dashboard';
import { Feed } from './pages/Feed';

const queryClient = new QueryClient();

function NavBar() {
  const linkStyle = ({ isActive }: { isActive: boolean }): React.CSSProperties => ({
    padding: '8px 18px',
    textDecoration: 'none',
    color: isActive ? '#4f46e5' : '#374151',
    fontWeight: isActive ? 600 : 400,
    borderBottom: isActive ? '2px solid #4f46e5' : '2px solid transparent',
  });

  return (
    <nav style={{
      display: 'flex', gap: '4px', padding: '0 24px',
      borderBottom: '1px solid #e5e7eb', background: '#fff',
      position: 'sticky', top: 0, zIndex: 10,
    }}>
      <NavLink to="/dashboard" style={linkStyle}>Dashboard</NavLink>
      <NavLink to="/feed" style={linkStyle}>Feed</NavLink>
    </nav>
  );
}

export default function App() {
  return (
    <QueryClientProvider client={queryClient}>
      <BrowserRouter>
        <NavBar />
        <div style={{ background: '#f9fafb', minHeight: 'calc(100vh - 42px)' }}>
          <Routes>
            <Route path="/" element={<Navigate to="/dashboard" replace />} />
            <Route path="/dashboard" element={<Dashboard />} />
            <Route path="/feed" element={<Feed />} />
          </Routes>
        </div>
      </BrowserRouter>
    </QueryClientProvider>
  );
}
