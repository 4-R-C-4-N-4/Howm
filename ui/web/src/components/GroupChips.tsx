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
    <div className="flex flex-wrap gap-1.5 items-center">
      <span className="text-xs text-howm-text-muted mr-1">Groups:</span>
      {groups.map(g => (
        <span key={g.group_id} className="inline-flex items-center gap-1 py-0.5 px-2.5 rounded text-xs bg-howm-accent-dim text-howm-accent border border-[rgba(59,130,246,0.25)]">
          {g.name}
          {!disabled && (
            <button onClick={() => onRemove(g.group_id)} className="bg-transparent border-none text-inherit cursor-pointer px-0.5 text-xs opacity-70">✕</button>
          )}
        </span>
      ))}
      <div className="relative" ref={dropRef}>
        <button
          onClick={() => setShowAdd(!showAdd)}
          disabled={disabled}
          className="bg-howm-accent-dim border border-[rgba(59,130,246,0.25)] text-howm-accent rounded w-[26px] h-[26px] cursor-pointer text-sm flex items-center justify-center"
        >
          +
        </button>
        {showAdd && (
          <div className="absolute left-0 top-full mt-1 bg-howm-bg-surface border border-howm-border rounded-lg z-200 min-w-[200px] shadow-[0_8px_24px_rgba(0,0,0,0.5)] overflow-hidden">
            <input
              autoFocus
              placeholder="Search groups..."
              value={search}
              onChange={e => setSearch(e.target.value)}
              className="w-full px-3 py-2 border-none border-b border-howm-border bg-transparent text-howm-text-primary text-sm outline-none box-border"
            />
            {available.length === 0 ? (
              <div className="px-3 py-2 text-howm-text-muted text-xs">No groups available</div>
            ) : available.map(g => (
              <button
                key={g.group_id}
                onClick={() => { onAdd(g.group_id); setShowAdd(false); setSearch(''); }}
                className="block w-full text-left bg-transparent border-none px-3 py-2 cursor-pointer text-sm text-howm-text-primary hover:bg-howm-bg-elevated"
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
