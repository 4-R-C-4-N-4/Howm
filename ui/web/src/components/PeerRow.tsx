import { useNavigate } from 'react-router-dom';
import { useState, useRef, useEffect } from 'react';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { movePeerToTier, denyPeer } from '../api/access';
import {
  effectiveTier, peerIdToHex, formatLastSeen, isOnline,
  GROUP_DEFAULT, GROUP_FRIENDS, GROUP_TRUSTED,
} from '../lib/access';
import type { AccessGroup } from '../api/access';
import type { Peer } from '../api/nodes';

interface PeerRowProps {
  peer: Peer;
  groups: AccessGroup[];
  now: number;
  onToast?: (level: 'success' | 'error', msg: string) => void;
}

export function PeerRow({ peer, groups, now, onToast }: PeerRowProps) {
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const [menuOpen, setMenuOpen] = useState(false);
  const menuRef = useRef<HTMLDivElement>(null);
  const hexId = peerIdToHex(peer.wg_pubkey);
  const badge = effectiveTier(groups);
  const online = isOnline(peer.last_seen, now);
  const isDenied = badge.label === 'Denied';

  useEffect(() => {
    if (!menuOpen) return;
    const handler = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) setMenuOpen(false);
    };
    document.addEventListener('mousedown', handler);
    return () => document.removeEventListener('mousedown', handler);
  }, [menuOpen]);

  const moveMutation = useMutation({
    mutationFn: (targetGroupId: string) => movePeerToTier(hexId, targetGroupId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['peer-groups'] });
      queryClient.invalidateQueries({ queryKey: ['peers'] });
      onToast?.('success', 'Permissions updated');
    },
    onError: () => onToast?.('error', 'Failed to update permissions'),
  });

  const denyMutation = useMutation({
    mutationFn: () => denyPeer(hexId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['peer-groups'] });
      queryClient.invalidateQueries({ queryKey: ['peers'] });
      onToast?.('success', `${peer.name} has been denied`);
    },
    onError: () => onToast?.('error', 'Failed to deny peer'),
  });

  const currentTier = badge.label;
  const tierOptions = [
    { label: 'Move to Trusted', group: GROUP_TRUSTED, tier: 'Trusted' },
    { label: 'Move to Friends', group: GROUP_FRIENDS, tier: 'Friends' },
    { label: 'Move to Default', group: GROUP_DEFAULT, tier: 'Default' },
  ];

  return (
    <div
      style={rowStyle}
      onClick={() => navigate(`/peers/${hexId}`)}
      onMouseEnter={e => (e.currentTarget.style.background = 'var(--howm-bg-elevated, #2a2e3d)')}
      onMouseLeave={e => (e.currentTarget.style.background = 'var(--howm-bg-secondary, #1a1d27)')}
    >
      <div style={{ display: 'flex', alignItems: 'center', gap: '10px', flex: 1, minWidth: 0 }}>
        <span style={{ color: isDenied ? '#f87171' : online ? '#4ade80' : '#5c6170', fontSize: '0.9rem' }}>
          {isDenied ? '✕' : online ? '●' : '○'}
        </span>
        <span style={{ fontWeight: 500, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', maxWidth: '180px' }}>
          {peer.name}
        </span>
        <span style={{
          fontSize: '0.75rem', padding: '2px 8px', borderRadius: '4px',
          background: badge.bg, color: badge.color, whiteSpace: 'nowrap',
        }}>
          {badge.label}
        </span>
        <span style={{ color: 'var(--howm-text-muted, #5c6170)', fontSize: '0.8rem', marginLeft: 'auto', whiteSpace: 'nowrap' }}>
          {isDenied ? '—' : formatLastSeen(peer.last_seen, now)}
        </span>
      </div>

      <div style={{ position: 'relative' }} ref={menuRef}>
        <button
          onClick={e => { e.stopPropagation(); setMenuOpen(!menuOpen); }}
          style={overflowBtnStyle}
        >
          ⋯
        </button>
        {menuOpen && (
          <div style={menuStyle} onClick={e => e.stopPropagation()}>
            {tierOptions.map(opt => {
              const isCurrent = opt.tier === currentTier;
              return (
                <button
                  key={opt.group}
                  disabled={isCurrent || moveMutation.isPending}
                  onClick={() => { moveMutation.mutate(opt.group); setMenuOpen(false); }}
                  style={{ ...menuItemStyle, color: isCurrent ? 'var(--howm-text-muted, #5c6170)' : 'var(--howm-text-primary, #e1e4eb)' }}
                >
                  {isCurrent ? `✓ ${opt.tier}` : opt.label}
                </button>
              );
            })}
            <div style={{ borderTop: '1px solid var(--howm-border, #2e3341)', margin: '4px 0' }} />
            <button
              onClick={() => { denyMutation.mutate(); setMenuOpen(false); }}
              disabled={denyMutation.isPending}
              style={{ ...menuItemStyle, color: '#f87171' }}
            >
              🔴 Deny Peer
            </button>
          </div>
        )}
      </div>
    </div>
  );
}

const rowStyle: React.CSSProperties = {
  display: 'flex', alignItems: 'center', gap: '8px',
  padding: '10px 12px',
  border: '1px solid var(--howm-border, #2e3341)',
  borderRadius: 'var(--howm-radius-sm, 4px)',
  marginBottom: '4px',
  background: 'var(--howm-bg-secondary, #1a1d27)',
  cursor: 'pointer',
  transition: 'background 0.15s',
};

const overflowBtnStyle: React.CSSProperties = {
  background: 'none', border: 'none', color: 'var(--howm-text-muted, #5c6170)',
  cursor: 'pointer', fontSize: '1.2rem', padding: '4px 8px', borderRadius: '4px',
};

const menuStyle: React.CSSProperties = {
  position: 'absolute', right: 0, top: '100%', marginTop: '4px',
  background: 'var(--howm-bg-surface, #232733)',
  border: '1px solid var(--howm-border, #2e3341)',
  borderRadius: '8px', padding: '4px 0', zIndex: 200,
  minWidth: '180px', boxShadow: '0 8px 24px rgba(0,0,0,0.5)',
};

const menuItemStyle: React.CSSProperties = {
  display: 'block', width: '100%', textAlign: 'left',
  background: 'none', border: 'none', padding: '8px 14px',
  cursor: 'pointer', fontSize: '0.875rem',
  color: 'var(--howm-text-primary, #e1e4eb)',
};
