import { BrowserRouter, Routes, Route, Navigate, NavLink, useNavigate } from 'react-router-dom';
import { QueryClient, QueryClientProvider, useQuery } from '@tanstack/react-query';
import { useEffect, useState, useCallback, useRef } from 'react';
import { Dashboard } from './pages/Dashboard';
import { Connection } from './pages/Connection';
import { Settings } from './pages/Settings';
import { CapabilityPage } from './pages/CapabilityPage';
import { PeersPage } from './pages/PeersPage';
import { PeerDetail } from './pages/PeerDetail';
import { GroupsPage } from './pages/GroupsPage';
import { GroupDetail } from './pages/GroupDetail';
import { getCapabilities } from './api/capabilities';
import { getBadges, pollNotifications } from './api/notifications';
import { getApiToken } from './api/client';
import api from './api/client';
import { listenFromCapabilities, type NotifyLevel } from './lib/postMessage';
import { useBadgeStore } from './stores/badgeStore';
import { FabLayer } from './components/FabLayer';

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
    <div className='fixed bottom-6 right-6 flex flex-col gap-2 z-300'>
      {toasts.map(t => (
        <div key={t.id} className='py-2.5 px-4 rounded-lg text-sm cursor-pointer max-w-80 shadow-[0_4px_12px_rgba(0,0,0,0.5)]' style={toastLevelStyle[t.level]} onClick={() => dismiss(t.id)}>
          {t.message}
        </div>
      ))}
    </div>
  );
}

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

  // Badge counts from Notification API (capabilities push their own badge counts)
  const { data: badgeData } = useQuery({
    queryKey: ['badges'],
    queryFn: getBadges,
    refetchInterval: 5_000,
  });
  const badges = badgeData?.badges ?? {};

  const linkClass = ({ isActive }: { isActive: boolean }) =>
    `px-4 h-12 flex items-center no-underline text-sm whitespace-nowrap border-b-2 ${
      isActive
        ? 'text-howm-accent font-semibold border-howm-accent'
        : 'text-howm-text-secondary font-normal border-transparent'
    }`;

  return (
    <nav className='flex items-center h-12 border-b border-howm-border bg-howm-bg-surface sticky top-0 z-100 pr-2 overflow-hidden'>
      <span className='px-5 font-bold text-base text-howm-accent tracking-wide border-r border-howm-border h-full flex items-center'>howm</span>
      <NavLink to="/dashboard" className={linkClass}>Dashboard</NavLink>
      <NavLink to="/peers" className={linkClass}>Peers</NavLink>
      <NavLink to="/connection" className={linkClass}>Connection</NavLink>
      <NavLink to="/access/groups" className={linkClass}>Groups</NavLink>
      {capabilities?.filter(c => c.ui && (!c.ui.style || c.ui.style === 'nav')).map(cap => {
        const badgeCount = badges[cap.name] ?? 0;
        return (
          <NavLink key={cap.name} to={`/app/${cap.name}`} className={linkClass}>
            {({ isActive }) => (
              <span className={`flex items-center gap-1.5 ${isActive ? 'text-howm-accent font-semibold' : 'text-inherit font-normal'}`}>
                {cap.ui!.label}
                {badgeCount > 0 && (
                  <span className='bg-howm-error text-white rounded-xl py-px px-1.5 text-[0.7rem] font-semibold min-w-4 text-center'>
                    {badgeCount}
                  </span>
                )}
              </span>
            )}
          </NavLink>
        );
      })}
      <NavLink to="/settings" className={({ isActive }) => `${linkClass({ isActive })} ml-auto`}>
        {({ isActive }) => (
          <span className={isActive ? 'text-howm-accent font-semibold' : 'text-howm-text-secondary font-normal'}>
            Settings
          </span>
        )}
      </NavLink>
    </nav>
  );
}

// ── App shell ─────────────────────────────────────────────────────────────────

function Shell() {
  const [toasts, setToasts] = useState<Toast[]>([]);
  const token = getApiToken();
  const navigate = useNavigate();
  const setBadge = useBadgeStore((s) => s.setBadge);

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
      {
        onNotify: (level, message) => addToast(level, message),
        onNavigate: (path) => navigate(path),
        onNavigateTo: (path) => navigate(path),
        onBadge: (capability, count) => setBadge(capability, count),
        onToast: (title, body) => {
          const message = title ? `${title}: ${body}` : body;
          addToast('info', message);
        },
      },
      token,
    );
  }, [token, addToast, navigate, setBadge]);

  // Poll daemon Notification API for capability-pushed toasts
  const notifCursor = useRef(0);
  useEffect(() => {
    const interval = setInterval(async () => {
      try {
        const resp = await pollNotifications(notifCursor.current);
        notifCursor.current = resp.timestamp;
        for (const n of resp.notifications) {
          addToast(n.level, n.title ? `${n.title}: ${n.message}` : n.message);
          if (n.action) {
            // If toast has an action, navigate on next user interaction
            // (for now just log — clicking toasts to navigate is handled by ToastContainer)
          }
        }
      } catch {
        // Notification polling is best-effort
      }
    }, 5_000);
    return () => clearInterval(interval);
  }, [addToast]);

  // Presence heartbeat — signals active status while tab is focused
  useEffect(() => {
    const interval = setInterval(() => {
      if (document.hasFocus()) {
        api.post('/cap/presence/heartbeat').catch(() => {});
      }
    }, 30_000);
    // Send an immediate heartbeat on mount
    if (document.hasFocus()) {
      api.post('/cap/presence/heartbeat').catch(() => {});
    }
    return () => clearInterval(interval);
  }, []);

  return (
    <>
      <NavBar />
      <div className='bg-howm-bg-primary min-h-[calc(100vh-48px)]'>
        <Routes>
          <Route path="/" element={<Navigate to="/dashboard" replace />} />
          <Route path="/dashboard" element={<Dashboard />} />
          <Route path="/peers" element={<PeersPage />} />
          <Route path="/peers/:peerId" element={<PeerDetail />} />
          <Route path="/connection" element={<Connection />} />
          <Route path="/access/groups" element={<GroupsPage />} />
          <Route path="/access/groups/:groupId" element={<GroupDetail />} />
          <Route path="/settings" element={<Settings />} />
          <Route path="/app/:name" element={<CapabilityPage />} />
        </Routes>
      </div>
      <FabLayer />
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
