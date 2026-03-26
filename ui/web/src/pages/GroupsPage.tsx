import { useQuery } from '@tanstack/react-query';
import { useState, useCallback, useRef } from 'react';
import { Link } from 'react-router-dom';
import { getAccessGroups } from '../api/access';
import { GROUP_DEFAULT, GROUP_FRIENDS, GROUP_TRUSTED } from '../lib/access';
import { CreateGroupModal } from '../components/CreateGroupModal';

const tierColors: Record<string, string> = {
  [GROUP_DEFAULT]: '#9ca3af',
  [GROUP_FRIENDS]: '#60a5fa',
  [GROUP_TRUSTED]: '#eab308',
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
    <div className='max-w-[800px] mx-auto p-6'>
      <div className='flex justify-between items-center mb-5'>
        <h1 className='text-2xl font-semibold m-0'>Access Groups</h1>
        <button onClick={() => setShowCreate(true)} className='py-2 px-4 bg-howm-accent border-none rounded-md text-white cursor-pointer text-sm font-semibold'>+ Create Group</button>
      </div>

      {/* Built-in */}
      <section className='bg-howm-bg-surface border border-howm-border rounded-xl p-4 mb-4'>
        <h3 className='text-xs font-semibold uppercase text-howm-text-muted m-0 mb-2.5 tracking-wide'>Built-in</h3>
        {builtIn.map(g => (
          <Link key={g.group_id} to={`/access/groups/${g.group_id}`} className='no-underline text-inherit'>
            <div className='flex items-center py-2.5 px-3 rounded cursor-pointer transition-colors duration-150 bg-howm-bg-secondary mb-1 hover:bg-howm-bg-elevated'>
              <span style={{ color: tierColors[g.group_id] || '#c084fc' }} className='mr-2.5'>●</span>
              <span className='font-medium flex-1'>{g.name}</span>
              <span className='text-xs text-howm-text-muted'>{g.capabilities?.length || 0} capabilities</span>
            </div>
          </Link>
        ))}
      </section>

      {/* Custom */}
      {custom.length > 0 && (
        <section className='bg-howm-bg-surface border border-howm-border rounded-xl p-4 mb-4'>
          <h3 className='text-xs font-semibold uppercase text-howm-text-muted m-0 mb-2.5 tracking-wide'>Custom</h3>
          {custom.map(g => (
            <Link key={g.group_id} to={`/access/groups/${g.group_id}`} className='no-underline text-inherit'>
              <div className='flex items-center py-2.5 px-3 rounded cursor-pointer transition-colors duration-150 bg-howm-bg-secondary mb-1 hover:bg-howm-bg-elevated'>
                <span className='text-purple-400 mr-2.5'>●</span>
                <span className='font-medium flex-1'>{g.name}</span>
                <span className='text-xs text-howm-text-muted'>{g.capabilities?.length || 0} capabilities</span>
              </div>
            </Link>
          ))}
        </section>
      )}

      {showCreate && (
        <CreateGroupModal onClose={() => setShowCreate(false)} onToast={showToast} />
      )}

      {toasts.length > 0 && (
        <div className='fixed bottom-6 left-1/2 -translate-x-1/2 flex flex-col gap-2 z-300'>
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
