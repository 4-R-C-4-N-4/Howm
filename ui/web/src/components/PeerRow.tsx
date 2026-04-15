import { useNavigate } from 'react-router-dom';
import { useState, useRef, useEffect } from 'react';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { movePeerToTier, denyPeer } from '../api/access';
import { removePeer } from '../api/nodes';
import {
  effectiveTier, peerIdToHex, formatLastSeen, isOnline,
  GROUP_DEFAULT, GROUP_FRIENDS, GROUP_TRUSTED,
} from '../lib/access';
import type { AccessGroup } from '../api/access';
import type { Peer } from '../api/nodes';
import type { PeerPresenceInfo } from '../pages/PeersPage';

interface PeerRowProps {
  peer: Peer;
  groups: AccessGroup[];
  now: number;
  onToast?: (level: 'success' | 'error', msg: string) => void;
  presence?: PeerPresenceInfo;
}

export function PeerRow({ peer, groups, now, onToast, presence }: PeerRowProps) {
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

  const forgetMutation = useMutation({
    mutationFn: () => removePeer(peer.node_id),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['peer-groups'] });
      queryClient.invalidateQueries({ queryKey: ['peers'] });
      onToast?.('success', `${peer.name} forgotten`);
    },
    onError: () => onToast?.('error', 'Failed to forget peer'),
  });

  const handleForget = () => {
    if (confirm(`Forget ${peer.name}? This removes the peer from peers.json, all access groups, the WireGuard interface, and any active session. They can be re-invited later.`)) {
      forgetMutation.mutate();
    }
  };

  const currentTier = badge.label;
  const tierOptions = [
    { label: 'Move to Trusted', group: GROUP_TRUSTED, tier: 'Trusted' },
    { label: 'Move to Friends', group: GROUP_FRIENDS, tier: 'Friends' },
    { label: 'Move to Default', group: GROUP_DEFAULT, tier: 'Default' },
  ];

  return (
    <div
      className='flex items-center gap-2 py-2.5 px-3 border border-howm-border rounded mb-1 bg-howm-bg-secondary cursor-pointer transition-colors duration-150 hover:bg-howm-bg-elevated'
      onClick={() => navigate(`/peers/${hexId}`)}
    >
      <div className='flex items-center gap-2.5 flex-1 min-w-0'>
        <span style={{ color: isDenied ? '#ef4444' : online ? (presence?.activity === 'away' ? '#eab308' : '#22c55e') : '#666666' }} className='text-sm'>
          {isDenied ? '✕' : online ? '●' : '○'}
        </span>
        <span className='font-medium overflow-hidden text-ellipsis whitespace-nowrap max-w-[180px]'>
          {peer.name}
        </span>
        {presence?.emoji && (
          <span className='text-sm'>{presence.emoji}</span>
        )}
        {presence?.status && (
          <span className='text-howm-text-muted text-xs overflow-hidden text-ellipsis whitespace-nowrap max-w-[160px]'>
            {presence.status}
          </span>
        )}
        <span style={{
          fontSize: '0.75rem', padding: '2px 8px', borderRadius: '4px',
          background: badge.bg, color: badge.color, whiteSpace: 'nowrap',
        }}>
          {badge.label}
        </span>
        <span className='text-howm-text-muted text-xs ml-auto whitespace-nowrap'>
          {isDenied ? '—' : formatLastSeen(peer.last_seen, now)}
        </span>
      </div>

      <div className='relative' ref={menuRef}>
        <button
          onClick={e => { e.stopPropagation(); setMenuOpen(!menuOpen); }}
          className='bg-none border-none text-howm-text-muted cursor-pointer text-lg py-1 px-2 rounded'
        >
          ⋯
        </button>
        {menuOpen && (
          <div className='absolute right-0 top-full mt-1 bg-howm-bg-surface border border-howm-border rounded-lg py-1 z-200 min-w-[180px] shadow-xl' onClick={e => e.stopPropagation()}>
            {tierOptions.map(opt => {
              const isCurrent = opt.tier === currentTier;
              return (
                <button
                  key={opt.group}
                  disabled={isCurrent || moveMutation.isPending}
                  onClick={() => { moveMutation.mutate(opt.group); setMenuOpen(false); }}
                  className={`block w-full text-left bg-none border-none py-2 px-3.5 cursor-pointer text-sm ${isCurrent ? 'text-howm-text-muted' : 'text-howm-text-primary'}`}
                >
                  {isCurrent ? `✓ ${opt.tier}` : opt.label}
                </button>
              );
            })}
            <div className='border-t border-howm-border my-1' />
            <button
              onClick={() => { denyMutation.mutate(); setMenuOpen(false); }}
              disabled={denyMutation.isPending}
              className='block w-full text-left bg-none border-none py-2 px-3.5 cursor-pointer text-sm text-red-500'
            >
              🔴 Deny Peer
            </button>
            <button
              onClick={() => { handleForget(); setMenuOpen(false); }}
              disabled={forgetMutation.isPending}
              className='block w-full text-left bg-none border-none py-2 px-3.5 cursor-pointer text-sm text-red-500'
            >
              🗑 Forget Peer
            </button>
          </div>
        )}
      </div>
    </div>
  );
}
