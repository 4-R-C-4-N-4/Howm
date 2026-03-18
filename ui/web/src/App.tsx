import { BrowserRouter, Routes, Route, Navigate, NavLink } from 'react-router-dom';
import { QueryClient, QueryClientProvider, useQuery } from '@tanstack/react-query';
import { useEffect, useState, useCallback } from 'react';
import { Dashboard } from './pages/Dashboard';
import { Settings } from './pages/Settings';
import { CapabilityPage } from './pages/CapabilityPage';
import { getCapabilities } from './api/capabilities';
import { getApiToken } from './api/client';
import { listenFromCapabilities, type NotifyLevel } from './lib/postMessage';

const queryClient = new QueryClient();

// ── Toast notifications ───────────────────────────────────────────────────────

interface Toast {
  id: number;
  level: NotifyLevel;
  message: string;
}

let _toastId = 0;

function ToastContainer({ toasts, dismiss }: { toasts: Toast[]; dismiss: (id: number) => void }) {
  if (!toasts.length) return null;
  return (
    <div style={toastContainerStyle}>
      {toasts.map(t => (
        <div key={t.id} style={{ ...toastStyle, ...toastLevelStyle[t.level] }} onClick={() => dismiss(t.id)}>
          {t.message}
        </div>
      ))}
    </div>
  );
}

const toastContainerStyle: React.CSSProperties = {
  position: 'fixed',
  bottom: '24px',
  right: '24px',
  display: 'flex',
  flexDirection: 'column',
  gap: '8px',
  zIndex: 300,
};
const toastStyle: React.CSSProperties = {
  padding: '10px 16px',
  borderRadius: '8px',
  fontSize: '0.875rem',
  cursor: 'pointer',
  maxWidth: '320px',
  boxShadow: '0 4px 12px rgba(0,0,0,0.5)',
};
const toastLevelStyle: Record<NotifyLevel, React.CSSProperties> = {
  info:    { background: '#1e3a5f', color: '#93c5fd', border: '1px solid #2563eb' },
  success: { background: '#14532d', color: '#86efac', border: '1px solid #16a34a' },
  warning: { background: '#78350f', color: '#fcd34d', border: '1px solid #d97706' },
  error:   { background: '#7f1d1d', color: '#fca5a5', border: '1px solid #dc2626' },
};

// ── Nav bar ───────────────────────────────────────────────────────────────────

function NavBar() {
  const { data: capabilities } = useQuery({
    queryKey: ['capabilities'],
    queryFn: getCapabilities,
    refetchInterval: 60000,
  });

  const linkStyle = ({ isActive }: { isActive: boolean }): React.CSSProperties => ({
    padding: '0 16px',
    height: '48px',
    display: 'flex',
    alignItems: 'center',
    textDecoration: 'none',
    color: isActive
      ? 'var(--howm-accent, #6c8cff)'
      : 'var(--howm-text-secondary, #8b91a0)',
    fontWeight: isActive ? 600 : 400,
    fontSize: '0.9rem',
    borderBottom: isActive ? '2px solid var(--howm-accent, #6c8cff)' : '2px solid transparent',
    whiteSpace: 'nowrap',
  });

  return (
    <nav style={navStyle}>
      <span style={brandStyle}>howm</span>
      <NavLink to="/dashboard" style={linkStyle}>Dashboard</NavLink>
      {capabilities?.filter(c => c.ui).map(cap => (
        <NavLink key={cap.name} to={`/cap/${cap.name}`} style={linkStyle}>
          {cap.ui!.label}
        </NavLink>
      ))}
      <NavLink to="/settings" style={{ ...linkStyle({ isActive: false }), marginLeft: 'auto' }}
        className={({ isActive }) => isActive ? 'active' : ''}>
        {({ isActive }) => (
          <span style={{
            color: isActive ? 'var(--howm-accent, #6c8cff)' : 'var(--howm-text-secondary, #8b91a0)',
            fontWeight: isActive ? 600 : 400,
          }}>
            Settings
          </span>
        )}
      </NavLink>
    </nav>
  );
}

const navStyle: React.CSSProperties = {
  display: 'flex',
  alignItems: 'center',
  height: '48px',
  borderBottom: '1px solid var(--howm-border, #2e3341)',
  background: 'var(--howm-bg-surface, #232733)',
  position: 'sticky',
  top: 0,
  zIndex: 100,
  paddingRight: '8px',
  overflow: 'hidden',
};
const brandStyle: React.CSSProperties = {
  padding: '0 20px',
  fontWeight: 700,
  fontSize: '1rem',
  color: 'var(--howm-accent, #6c8cff)',
  letterSpacing: '0.04em',
  borderRight: '1px solid var(--howm-border, #2e3341)',
  height: '100%',
  display: 'flex',
  alignItems: 'center',
};

// ── App shell ─────────────────────────────────────────────────────────────────

function Shell() {
  const [toasts, setToasts] = useState<Toast[]>([]);
  const token = getApiToken();

  const addToast = useCallback((level: NotifyLevel, message: string) => {
    const id = ++_toastId;
    setToasts(prev => [...prev, { id, level, message }]);
    setTimeout(() => setToasts(prev => prev.filter(t => t.id !== id)), 5000);
  }, []);

  const dismissToast = useCallback((id: number) => {
    setToasts(prev => prev.filter(t => t.id !== id));
  }, []);

  useEffect(() => {
    return listenFromCapabilities(
      { onNotify: (level, message) => addToast(level, message) },
      token,
    );
  }, [token, addToast]);

  return (
    <>
      <NavBar />
      <div style={{ background: 'var(--howm-bg-primary, #0f1117)', minHeight: 'calc(100vh - 48px)' }}>
        <Routes>
          <Route path="/" element={<Navigate to="/dashboard" replace />} />
          <Route path="/dashboard" element={<Dashboard />} />
          <Route path="/settings" element={<Settings />} />
          <Route path="/cap/:name" element={<CapabilityPage />} />
        </Routes>
      </div>
      <ToastContainer toasts={toasts} dismiss={dismissToast} />
    </>
  );
}

export default function App() {
  return (
    <QueryClientProvider client={queryClient}>
      <BrowserRouter>
        <Shell />
      </BrowserRouter>
    </QueryClientProvider>
  );
}
