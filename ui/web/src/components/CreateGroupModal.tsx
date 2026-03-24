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
    <div style={overlayStyle} onClick={onClose}>
      <div style={modalStyle} onClick={e => e.stopPropagation()}>
        <h3 style={{ margin: '0 0 16px', fontSize: '1.1rem' }}>Create Access Group</h3>

        <div style={{ marginBottom: '12px' }}>
          <label style={labelStyle}>Name</label>
          <input
            value={name} onChange={e => setName(e.target.value)}
            placeholder="my-custom-group"
            style={inputStyle}
            maxLength={64}
          />
        </div>

        <div style={{ marginBottom: '16px' }}>
          <label style={labelStyle}>Description</label>
          <input
            value={description} onChange={e => setDescription(e.target.value)}
            placeholder="Optional description"
            style={inputStyle}
          />
        </div>

        <div style={sectionStyle}>
          <label style={labelStyle}>Capabilities</label>
          <div style={{ display: 'flex', flexDirection: 'column', gap: '4px', marginBottom: '12px' }}>
            {ALL_CAPABILITIES.map(cap => (
              <label key={cap} style={{ display: 'flex', alignItems: 'center', gap: '8px', cursor: 'pointer', fontSize: '0.875rem' }}>
                <input
                  type="checkbox"
                  checked={selected.has(cap)}
                  onChange={() => toggle(cap)}
                  style={{ accentColor: 'var(--howm-accent, #6c8cff)' }}
                />
                <span style={{ fontFamily: 'var(--howm-font-mono, monospace)' }}>{cap}</span>
                {isCore(cap) && <span style={{ fontSize: '0.7rem', color: 'var(--howm-text-muted, #5c6170)' }}>core</span>}
              </label>
            ))}
          </div>

          <div style={{ display: 'flex', gap: '6px', flexWrap: 'wrap' }}>
            <span style={{ fontSize: '0.8rem', color: 'var(--howm-text-muted, #5c6170)', alignSelf: 'center' }}>Presets:</span>
            <button onClick={() => applyPreset(GROUP_DEFAULT)} style={presetBtnStyle}>Default</button>
            <button onClick={() => applyPreset(GROUP_FRIENDS)} style={presetBtnStyle}>Friends</button>
            <button onClick={() => applyPreset(GROUP_TRUSTED)} style={presetBtnStyle}>Trusted</button>
            <button onClick={() => applyPreset(null)} style={presetBtnStyle}>None</button>
          </div>
        </div>

        <div style={{ display: 'flex', gap: '8px', justifyContent: 'flex-end', marginTop: '20px' }}>
          <button onClick={onClose} style={cancelBtnStyle}>Cancel</button>
          <button
            onClick={() => mutation.mutate()}
            disabled={!valid || mutation.isPending}
            style={{ ...createBtnStyle, opacity: valid ? 1 : 0.5 }}
          >
            {mutation.isPending ? 'Creating…' : 'Create'}
          </button>
        </div>
      </div>
    </div>
  );
}

const overlayStyle: React.CSSProperties = {
  position: 'fixed', inset: 0, background: 'rgba(0,0,0,0.6)',
  display: 'flex', alignItems: 'center', justifyContent: 'center',
  zIndex: 250,
};
const modalStyle: React.CSSProperties = {
  background: 'var(--howm-bg-surface, #232733)',
  border: '1px solid var(--howm-border, #2e3341)',
  borderRadius: '12px', padding: '24px',
  maxWidth: '500px', width: '90%', maxHeight: '80vh', overflowY: 'auto',
  boxShadow: '0 16px 48px rgba(0,0,0,0.6)',
};
const labelStyle: React.CSSProperties = {
  display: 'block', fontSize: '0.8rem', fontWeight: 600,
  color: 'var(--howm-text-secondary, #8b91a0)', marginBottom: '6px',
};
const inputStyle: React.CSSProperties = {
  width: '100%', padding: '8px 10px', boxSizing: 'border-box',
  background: 'var(--howm-bg-secondary, #1a1d27)',
  border: '1px solid var(--howm-border, #2e3341)',
  borderRadius: '4px', color: 'var(--howm-text-primary, #e1e4eb)',
  fontSize: '0.9rem',
};
const sectionStyle: React.CSSProperties = {
  background: 'var(--howm-bg-secondary, #1a1d27)',
  border: '1px solid var(--howm-border, #2e3341)',
  borderRadius: '8px', padding: '12px',
};
const presetBtnStyle: React.CSSProperties = {
  padding: '4px 10px', background: 'var(--howm-bg-elevated, #2a2e3d)',
  border: '1px solid var(--howm-border, #2e3341)', borderRadius: '4px',
  color: 'var(--howm-text-primary, #e1e4eb)', cursor: 'pointer', fontSize: '0.75rem',
};
const cancelBtnStyle: React.CSSProperties = {
  padding: '8px 20px', background: 'var(--howm-bg-elevated, #2a2e3d)',
  border: '1px solid var(--howm-border, #2e3341)', borderRadius: '6px',
  color: 'var(--howm-text-primary, #e1e4eb)', cursor: 'pointer', fontSize: '0.9rem',
};
const createBtnStyle: React.CSSProperties = {
  padding: '8px 20px', background: 'var(--howm-accent, #6c8cff)',
  border: 'none', borderRadius: '6px', color: '#fff',
  cursor: 'pointer', fontSize: '0.9rem', fontWeight: 600,
};
