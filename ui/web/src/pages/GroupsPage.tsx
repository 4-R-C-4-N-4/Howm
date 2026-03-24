import { useQuery } from '@tanstack/react-query';
import { useState, useCallback, useRef } from 'react';
import { Link } from 'react-router-dom';
import { getAccessGroups } from '../api/access';
import { GROUP_DEFAULT, GROUP_FRIENDS, GROUP_TRUSTED } from '../lib/access';
import { CreateGroupModal } from '../components/CreateGroupModal';

const tierColors: Record<string, string> = {
  [GROUP_DEFAULT]: '#9ca3af',
  [GROUP_FRIENDS]: '#60a5fa',
  [GROUP_TRUSTED]: '#fbbf24',
};

export function GroupsPage() {
  const [showCreate, setShowCreate] = useState(false);

  const { data: groups = [] } = useQuery({
    queryKey: ['access-groups'],
    queryFn: getAccessGroups,
    refetchInterval: 60_000,
  });

  const [toasts, setToasts] = useState<{ id: number; level: string; msg: string }[]>([]);
  const toastId = useRef(0);
  const showToast = useCallback((level: 'success' | 'error', msg: string) => {
    const id = ++toastId.current;
    setToasts(prev => [...prev, { id, level, msg }]);
    setTimeout(() => setToasts(prev => prev.filter(t => t.id !== id)), 4000);
  }, []);

  const builtIn = groups
    .filter(g => g.built_in)
    .sort((a, b) => {
      const order = [GROUP_DEFAULT, GROUP_FRIENDS, GROUP_TRUSTED];
      return order.indexOf(a.group_id) - order.indexOf(b.group_id);
    });
  const custom = groups.filter(g => !g.built_in).sort((a, b) => a.name.localeCompare(b.name));

  return (
    <div style={pageStyle}>
      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '20px' }}>
        <h1 style={h1Style}>Access Groups</h1>
        <button onClick={() => setShowCreate(true)} style={createBtnStyle}>+ Create Group</button>
      </div>

      {/* Built-in */}
      <section style={sectionStyle}>
        <h3 style={sectionLabelStyle}>Built-in</h3>
        {builtIn.map(g => (
          <Link key={g.group_id} to={`/access/groups/${g.group_id}`} style={{ textDecoration: 'none', color: 'inherit' }}>
            <div
              style={groupRowStyle}
              onMouseEnter={e => (e.currentTarget.style.background = 'var(--howm-bg-elevated, #2a2e3d)')}
              onMouseLeave={e => (e.currentTarget.style.background = 'var(--howm-bg-secondary, #1a1d27)')}
            >
              <span style={{ color: tierColors[g.group_id] || '#c084fc', marginRight: '10px' }}>●</span>
              <span style={{ fontWeight: 500, flex: 1 }}>{g.name}</span>
              <span style={metaStyle}>{g.capabilities?.length || 0} capabilities</span>
            </div>
          </Link>
        ))}
      </section>

      {/* Custom */}
      {custom.length > 0 && (
        <section style={sectionStyle}>
          <h3 style={sectionLabelStyle}>Custom</h3>
          {custom.map(g => (
            <Link key={g.group_id} to={`/access/groups/${g.group_id}`} style={{ textDecoration: 'none', color: 'inherit' }}>
              <div
                style={groupRowStyle}
                onMouseEnter={e => (e.currentTarget.style.background = 'var(--howm-bg-elevated, #2a2e3d)')}
                onMouseLeave={e => (e.currentTarget.style.background = 'var(--howm-bg-secondary, #1a1d27)')}
              >
                <span style={{ color: '#c084fc', marginRight: '10px' }}>●</span>
                <span style={{ fontWeight: 500, flex: 1 }}>{g.name}</span>
                <span style={metaStyle}>{g.capabilities?.length || 0} capabilities</span>
              </div>
            </Link>
          ))}
        </section>
      )}

      {showCreate && (
        <CreateGroupModal onClose={() => setShowCreate(false)} onToast={showToast} />
      )}

      {toasts.length > 0 && (
        <div style={{ position: 'fixed', bottom: '24px', left: '50%', transform: 'translateX(-50%)', display: 'flex', flexDirection: 'column', gap: '8px', zIndex: 300 }}>
          {toasts.map(t => (
            <div key={t.id} style={{
              padding: '8px 16px', borderRadius: '8px', fontSize: '0.85rem',
              background: t.level === 'success' ? '#14532d' : '#7f1d1d',
              color: t.level === 'success' ? '#86efac' : '#fca5a5',
              border: `1px solid ${t.level === 'success' ? '#16a34a' : '#dc2626'}`,
              boxShadow: '0 4px 12px rgba(0,0,0,0.5)',
            }}>
              {t.msg}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

const pageStyle: React.CSSProperties = { maxWidth: '800px', margin: '0 auto', padding: '24px' };
const h1Style: React.CSSProperties = { fontSize: '1.5rem', fontWeight: 600, margin: 0 };
const sectionStyle: React.CSSProperties = {
  background: 'var(--howm-bg-surface, #232733)',
  border: '1px solid var(--howm-border, #2e3341)',
  borderRadius: '12px', padding: '16px', marginBottom: '16px',
};
const sectionLabelStyle: React.CSSProperties = {
  fontSize: '0.8rem', fontWeight: 600, textTransform: 'uppercase',
  color: 'var(--howm-text-muted, #5c6170)',
  margin: '0 0 10px', letterSpacing: '0.05em',
};
const groupRowStyle: React.CSSProperties = {
  display: 'flex', alignItems: 'center',
  padding: '10px 12px',
  borderRadius: '4px', cursor: 'pointer',
  transition: 'background 0.15s',
  background: 'var(--howm-bg-secondary, #1a1d27)',
  marginBottom: '4px',
};
const metaStyle: React.CSSProperties = {
  fontSize: '0.8rem', color: 'var(--howm-text-muted, #5c6170)',
};
const createBtnStyle: React.CSSProperties = {
  padding: '8px 16px', background: 'var(--howm-accent, #6c8cff)',
  border: 'none', borderRadius: '6px', color: '#fff',
  cursor: 'pointer', fontSize: '0.9rem', fontWeight: 600,
};
