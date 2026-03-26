import { useState } from 'react';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { createAccessGroup, TIER_CAPABILITIES, GROUP_DEFAULT, GROUP_FRIENDS, GROUP_TRUSTED } from '../api/access';
import { ALL_CAPABILITIES, CORE_CAPABILITIES } from '../lib/access';

interface CreateGroupModalProps {
  onClose: () => void;
  onToast?: (level: 'success' | 'error', msg: string) => void;
}

export function CreateGroupModal({ onClose, onToast }: CreateGroupModalProps) {
  const queryClient = useQueryClient();
  const [name, setName] = useState('');
  const [description, setDescription] = useState('');
  const [selected, setSelected] = useState<Set<string>>(new Set(CORE_CAPABILITIES));

  const mutation = useMutation({
    mutationFn: () => createAccessGroup(
      name.trim(),
      description.trim() || undefined,
      ALL_CAPABILITIES.map(cap => ({
        capability_name: cap,
        allow: selected.has(cap),
        rate_limit: null,
        ttl: null,
      })),
    ),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['access-groups'] });
      onToast?.('success', `Group "${name.trim()}" created`);
      onClose();
    },
    onError: () => onToast?.('error', 'Failed to create group'),
  });

  const toggle = (cap: string) => {
    const next = new Set(selected);
    if (next.has(cap)) { next.delete(cap); } else { next.add(cap); }
    setSelected(next);
  };

  const applyPreset = (groupId: string | null) => {
    if (groupId === null) { setSelected(new Set()); return; }
    setSelected(new Set(TIER_CAPABILITIES[groupId] || []));
  };

  const isCore = (cap: string) => CORE_CAPABILITIES.includes(cap);
  const valid = name.trim().length >= 1 && name.trim().length <= 64;

  return (
    <div className="fixed inset-0 bg-black/60 flex items-center justify-center z-[250]" onClick={onClose}>
      <div className="bg-howm-bg-surface border border-howm-border rounded-xl p-6 max-w-[500px] w-[90%] max-h-[80vh] overflow-y-auto shadow-[0_16px_48px_rgba(0,0,0,0.6)]" onClick={e => e.stopPropagation()}>
        <h3 className="m-0 mb-4 text-lg">Create Access Group</h3>

        <div className="mb-3">
          <label className="block text-xs font-semibold text-howm-text-secondary mb-1.5">Name</label>
          <input
            value={name} onChange={e => setName(e.target.value)}
            placeholder="my-custom-group"
            className="w-full p-2 bg-howm-bg-secondary border border-howm-border rounded text-howm-text-primary text-sm"
            maxLength={64}
          />
        </div>

        <div className="mb-4">
          <label className="block text-xs font-semibold text-howm-text-secondary mb-1.5">Description</label>
          <input
            value={description} onChange={e => setDescription(e.target.value)}
            placeholder="Optional description"
            className="w-full p-2 bg-howm-bg-secondary border border-howm-border rounded text-howm-text-primary text-sm"
          />
        </div>

        <div className="bg-howm-bg-secondary border border-howm-border rounded-lg p-3">
          <label className="block text-xs font-semibold text-howm-text-secondary mb-1.5">Capabilities</label>
          <div className="flex flex-col gap-1 mb-3">
            {ALL_CAPABILITIES.map(cap => (
              <label key={cap} className="flex items-center gap-2 cursor-pointer text-sm">
                <input
                  type="checkbox"
                  checked={selected.has(cap)}
                  onChange={() => toggle(cap)}
                  style={{ accentColor: 'var(--howm-accent, #3b82f6)' }}
                />
                <span className="font-mono">{cap}</span>
                {isCore(cap) && <span className="text-[0.7rem] text-howm-text-muted">core</span>}
              </label>
            ))}
          </div>

          <div className="flex gap-1.5 flex-wrap">
            <span className="text-xs text-howm-text-muted self-center">Presets:</span>
            <button onClick={() => applyPreset(GROUP_DEFAULT)} className="py-1 px-2.5 bg-howm-bg-elevated border border-howm-border rounded text-howm-text-primary cursor-pointer text-xs">Default</button>
            <button onClick={() => applyPreset(GROUP_FRIENDS)} className="py-1 px-2.5 bg-howm-bg-elevated border border-howm-border rounded text-howm-text-primary cursor-pointer text-xs">Friends</button>
            <button onClick={() => applyPreset(GROUP_TRUSTED)} className="py-1 px-2.5 bg-howm-bg-elevated border border-howm-border rounded text-howm-text-primary cursor-pointer text-xs">Trusted</button>
            <button onClick={() => applyPreset(null)} className="py-1 px-2.5 bg-howm-bg-elevated border border-howm-border rounded text-howm-text-primary cursor-pointer text-xs">None</button>
          </div>
        </div>

        <div className="flex gap-2 justify-end mt-5">
          <button onClick={onClose} className="py-2 px-5 bg-howm-bg-elevated border border-howm-border rounded-md text-howm-text-primary cursor-pointer text-sm">Cancel</button>
          <button
            onClick={() => mutation.mutate()}
            disabled={!valid || mutation.isPending}
            className="py-2 px-5 bg-howm-accent border-none rounded-md text-white cursor-pointer text-sm font-semibold"
            style={{ opacity: valid ? 1 : 0.5 }}
          >
            {mutation.isPending ? 'Creating…' : 'Create'}
          </button>
        </div>
      </div>
    </div>
  );
}
