import { useState, useRef, useEffect } from 'react';
import type { AccessGroup } from '../api/access';

interface GroupChipsProps {
  groups: AccessGroup[];
  allGroups: AccessGroup[];
  onRemove: (groupId: string) => void;
  onAdd: (groupId: string) => void;
  disabled?: boolean;
}

export function GroupChips({ groups, allGroups, onRemove, onAdd, disabled }: GroupChipsProps) {
  const [showAdd, setShowAdd] = useState(false);
  const [search, setSearch] = useState('');
  const dropRef = useRef<HTMLDivElement>(null);

  const memberIds = new Set(groups.map(g => g.group_id));
  const available = allGroups
    .filter(g => !memberIds.has(g.group_id))
    .filter(g => !search || g.name.toLowerCase().includes(search.toLowerCase()));

  useEffect(() => {
    if (!showAdd) return;
    const handler = (e: MouseEvent) => {
      if (dropRef.current && !dropRef.current.contains(e.target as Node)) { setShowAdd(false); setSearch(''); }
    };
    document.addEventListener('mousedown', handler);
    return () => document.removeEventListener('mousedown', handler);
  }, [showAdd]);

  return (
    <div style={{ display: 'flex', flexWrap: 'wrap', gap: '6px', alignItems: 'center' }}>
      <span style={{ fontSize: '0.8rem', color: 'var(--howm-text-muted, #5c6170)', marginRight: '4px' }}>Groups:</span>
      {groups.map(g => (
        <span key={g.group_id} style={chipStyle}>
          {g.name}
          {!disabled && (
            <button onClick={() => onRemove(g.group_id)} style={chipXStyle}>✕</button>
          )}
        </span>
      ))}
      <div style={{ position: 'relative' }} ref={dropRef}>
        <button
          onClick={() => setShowAdd(!showAdd)}
          disabled={disabled}
          style={addBtnStyle}
        >
          +
        </button>
        {showAdd && (
          <div style={dropdownStyle}>
            <input
              autoFocus
              placeholder="Search groups..."
              value={search}
              onChange={e => setSearch(e.target.value)}
              style={searchInputStyle}
            />
            {available.length === 0 ? (
              <div style={{ padding: '8px 12px', color: 'var(--howm-text-muted, #5c6170)', fontSize: '0.8rem' }}>No groups available</div>
            ) : available.map(g => (
              <button
                key={g.group_id}
                onClick={() => { onAdd(g.group_id); setShowAdd(false); setSearch(''); }}
                style={dropItemStyle}
              >
                {g.name}
              </button>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

const chipStyle: React.CSSProperties = {
  display: 'inline-flex', alignItems: 'center', gap: '4px',
  padding: '3px 10px', borderRadius: '4px', fontSize: '0.8rem',
  background: 'rgba(108,140,255,0.12)', color: 'var(--howm-accent, #6c8cff)',
  border: '1px solid rgba(108,140,255,0.25)',
};

const chipXStyle: React.CSSProperties = {
  background: 'none', border: 'none', color: 'inherit',
  cursor: 'pointer', padding: '0 2px', fontSize: '0.75rem', opacity: 0.7,
};

const addBtnStyle: React.CSSProperties = {
  background: 'rgba(108,140,255,0.12)', border: '1px solid rgba(108,140,255,0.25)',
  color: 'var(--howm-accent, #6c8cff)', borderRadius: '4px',
  width: '26px', height: '26px', cursor: 'pointer', fontSize: '0.9rem',
  display: 'flex', alignItems: 'center', justifyContent: 'center',
};

const dropdownStyle: React.CSSProperties = {
  position: 'absolute', left: 0, top: '100%', marginTop: '4px',
  background: 'var(--howm-bg-surface, #232733)',
  border: '1px solid var(--howm-border, #2e3341)',
  borderRadius: '8px', zIndex: 200, minWidth: '200px',
  boxShadow: '0 8px 24px rgba(0,0,0,0.5)', overflow: 'hidden',
};

const searchInputStyle: React.CSSProperties = {
  width: '100%', padding: '8px 12px', border: 'none',
  borderBottom: '1px solid var(--howm-border, #2e3341)',
  background: 'transparent', color: 'var(--howm-text-primary, #e1e4eb)',
  fontSize: '0.85rem', outline: 'none', boxSizing: 'border-box',
};

const dropItemStyle: React.CSSProperties = {
  display: 'block', width: '100%', textAlign: 'left',
  background: 'none', border: 'none', padding: '8px 12px',
  cursor: 'pointer', fontSize: '0.85rem',
  color: 'var(--howm-text-primary, #e1e4eb)',
};
