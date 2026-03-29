import { useParams, useNavigate, Link } from 'react-router-dom';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { useState, useCallback, useRef } from 'react';
import { getPeers } from '../api/nodes';
import {
  getPeerGroups, getPeerPermissions, getAccessGroups,
  movePeerToTier, denyPeer, assignPeerToGroup, removePeerFromGroup,
} from '../api/access';
import { getCachedProfile } from '../api/profile';
import { TierSelector } from '../components/TierSelector';
import { GroupChips } from '../components/GroupChips';
import { PermissionGrid } from '../components/PermissionGrid';
import { DemotionWarning } from '../components/DemotionWarning';
import { DenyModal } from '../components/DenyModal';
import {
  effectiveTier, peerIdToHex, formatLastSeen, isOnline,
  GROUP_DEFAULT, GROUP_FRIENDS, GROUP_TRUSTED, BUILT_IN_TIERS,
} from '../lib/access';

export function PeerDetail() {
  const { peerId } = useParams<{ peerId: string }>();
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const [showDenyModal, setShowDenyModal] = useState(false);
  const [demotionTarget, setDemotionTarget] = useState<string | null>(null);

  const [toasts, setToasts] = useState<{ id: number; level: string; msg: string }[]>([]);
  const toastId = useRef(0);
  const showToast = useCallback((level: 'success' | 'error', msg: string) => {
    const id = ++toastId.current;
    setToasts(prev => [...prev, { id, level, msg }]);
    setTimeout(() => setToasts(prev => prev.filter(t => t.id !== id)), 4000);
  }, []);

  // Fetch peers list and find this peer
  const { data: peers = [], dataUpdatedAt } = useQuery({
    queryKey: ['peers'],
    queryFn: getPeers,
    refetchInterval: 30_000,
  });

  const peer = peers.find(p => peerIdToHex(p.wg_pubkey) === peerId);

  const { data: peerGroups = [] } = useQuery({
    queryKey: ['peer-groups', peerId],
    queryFn: () => getPeerGroups(peerId!),
    enabled: !!peerId,
    staleTime: 60_000,
  });

  const { data: permissions, isLoading: permLoading } = useQuery({
    queryKey: ['peer-permissions', peerId],
    queryFn: () => getPeerPermissions(peerId!),
    enabled: !!peerId,
    staleTime: 60_000,
  });

  const { data: allGroups = [] } = useQuery({
    queryKey: ['access-groups'],
    queryFn: getAccessGroups,
    refetchInterval: 60_000,
  });

  // Cached profile data
  const { data: cachedProfile } = useQuery({
    queryKey: ['peer-profile-cache', peer?.node_id],
    queryFn: () => getCachedProfile(peer!.node_id),
    enabled: !!peer,
    staleTime: 60_000,
  });

  const badge = effectiveTier(peerGroups);
  const online = peer ? isOnline(peer.last_seen, dataUpdatedAt) : false;

  // Current tier group ID
  const builtInIds = new Set(peerGroups.filter(g => g.built_in).map(g => g.group_id));
  const currentTierGroupId =
    builtInIds.has(GROUP_TRUSTED) ? GROUP_TRUSTED :
    builtInIds.has(GROUP_FRIENDS) ? GROUP_FRIENDS :
    builtInIds.has(GROUP_DEFAULT) ? GROUP_DEFAULT : null;

  const invalidate = () => {
    queryClient.invalidateQueries({ queryKey: ['peer-groups', peerId] });
    queryClient.invalidateQueries({ queryKey: ['peer-permissions', peerId] });
  };

  const moveMutation = useMutation({
    mutationFn: (targetGroupId: string) => movePeerToTier(peerId!, targetGroupId),
    onSuccess: () => { invalidate(); showToast('success', 'Permissions updated'); setDemotionTarget(null); },
    onError: () => showToast('error', 'Failed to update permissions'),
  });

  const denyMutation = useMutation({
    mutationFn: () => denyPeer(peerId!),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['peers'] });
      navigate('/peers');
    },
    onError: () => showToast('error', 'Failed to deny peer'),
  });

  const addGroupMutation = useMutation({
    mutationFn: (groupId: string) => assignPeerToGroup(peerId!, groupId),
    onSuccess: () => { invalidate(); showToast('success', 'Group added'); },
    onError: () => showToast('error', 'Failed to add group'),
  });

  const removeGroupMutation = useMutation({
    mutationFn: (groupId: string) => removePeerFromGroup(peerId!, groupId),
    onSuccess: () => { invalidate(); showToast('success', 'Group removed'); },
    onError: () => showToast('error', 'Failed to remove group'),
  });

  const handleTierSelect = (targetGroupId: string) => {
    // Check if this is a demotion
    const currentOrder = currentTierGroupId
      ? BUILT_IN_TIERS.find(t => t.id === currentTierGroupId)?.order ?? -1
      : -1;
    const targetOrder = BUILT_IN_TIERS.find(t => t.id === targetGroupId)?.order ?? -1;

    if (targetOrder < currentOrder) {
      // Demotion — show warning
      setDemotionTarget(targetGroupId);
    } else {
      // Promotion — immediate
      moveMutation.mutate(targetGroupId);
    }
  };

  // Compute current capabilities for demotion diff
  const currentCaps = permissions
    ? Object.entries(permissions.permissions).filter(([, v]) => v.allowed).map(([k]) => k)
    : [];

  const copyToClipboard = (text: string) => {
    navigator.clipboard.writeText(text).then(
      () => showToast('success', 'Copied'),
      () => showToast('error', 'Copy failed'),
    );
  };

  if (!peer) {
    return (
      <div className='max-w-[800px] mx-auto p-6'>
        <Link to="/peers" className='text-howm-text-muted no-underline text-sm'>← Back to Peers</Link>
        <p className='text-howm-text-muted text-sm'>Peer not found</p>
      </div>
    );
  }

  return (
    <div className='max-w-[800px] mx-auto p-6'>
      <Link to="/peers" className='text-howm-text-muted no-underline text-sm'>← Back to Peers</Link>

      <div className='flex justify-between items-start mt-3 mb-2'>
        <div>
          <h1 className='m-0 text-2xl'>{peer.name}</h1>
          <span className='inline-block mt-1 text-sm py-0.5 px-2.5 rounded' style={{
            background: badge.bg, color: badge.color,
          }}>
            {badge.label}
          </span>
        </div>
        <div className='flex flex-col items-end gap-1.5'>
          <span className='text-sm' style={{ color: online ? '#22c55e' : '#666666' }}>
            {online ? '● Online' : '○ Offline'}
          </span>
          <Link
            to={`/app/social.messaging?peer=${encodeURIComponent(peer.wg_pubkey)}`}
            className='bg-howm-accent text-white border-none rounded-md py-1.5 px-3.5 text-sm font-semibold no-underline cursor-pointer'
          >
            Message
          </Link>
        </div>
      </div>

      {/* Profile Card */}
      <section className='bg-howm-bg-surface border border-howm-border rounded-xl p-5 mb-4'>
        <div className='flex items-center gap-4'>
          <div className='w-16 h-16 rounded-full bg-howm-bg-elevated border border-howm-border overflow-hidden flex items-center justify-center shrink-0'>
            <img
              src={`http://${peer.wg_address}:${peer.port}/profile/avatar`}
              alt=""
              className='w-full h-full object-cover'
              onError={(e) => { (e.target as HTMLImageElement).style.display = 'none'; }}
            />
            <span className='text-2xl text-howm-text-muted'>👤</span>
          </div>
          <div>
            <h2 className='m-0 text-lg font-semibold'>{peer.name}</h2>
            {cachedProfile?.found && cachedProfile.bio && (
              <p className='text-howm-text-secondary text-sm mt-1 mb-0'>{cachedProfile.bio}</p>
            )}
          </div>
        </div>
      </section>

      {/* Homepage */}
      {cachedProfile?.found && cachedProfile.has_homepage && (
        <section className='bg-howm-bg-surface border border-howm-border rounded-xl p-5 mb-4'>
          <h3 className='text-base font-semibold m-0 mb-3 text-howm-text-secondary'>Homepage</h3>
          <div className='border border-howm-border rounded overflow-hidden bg-white' style={{ height: 400 }}>
            <iframe
              src={`http://${peer.wg_address}:${peer.port}/profile/home`}
              title={`${peer.name}'s homepage`}
              sandbox="allow-scripts"
              className='w-full h-full border-none'
            />
          </div>
          <a
            href={`http://${peer.wg_address}:${peer.port}/profile/home`}
            target="_blank"
            rel="noopener noreferrer"
            className='inline-block mt-2 text-howm-accent text-sm no-underline'
          >
            Open in new tab ↗
          </a>
        </section>
      )}

      {/* Identity */}
      <section className='bg-howm-bg-surface border border-howm-border rounded-xl p-5 mb-4'>
        <h3 className='text-base font-semibold m-0 mb-3 text-howm-text-secondary'>Identity</h3>
        <dl className='grid grid-cols-[auto_1fr] gap-x-4 gap-y-1.5 m-0'>
          <dt className='font-semibold text-howm-text-secondary text-sm'>Node ID</dt>
          <dd className='m-0 text-sm flex items-center gap-2'>
            <code className='font-mono break-all text-sm'>{peerId!.slice(0, 16)}…{peerId!.slice(-8)}</code>
            <button onClick={() => copyToClipboard(peerId!)} className='bg-transparent border border-howm-border rounded-sm text-howm-text-muted cursor-pointer text-[0.7rem] py-px px-1.5'>copy</button>
          </dd>
          <dt className='font-semibold text-howm-text-secondary text-sm'>WG Pubkey</dt>
          <dd className='m-0 text-sm flex items-center gap-2'>
            <code className='font-mono break-all text-sm'>{peer.wg_pubkey}</code>
            <button onClick={() => copyToClipboard(peer.wg_pubkey)} className='bg-transparent border border-howm-border rounded-sm text-howm-text-muted cursor-pointer text-[0.7rem] py-px px-1.5'>copy</button>
          </dd>
          <dt className='font-semibold text-howm-text-secondary text-sm'>WG Address</dt>
          <dd className='m-0 text-sm'>{peer.wg_address}</dd>
          <dt className='font-semibold text-howm-text-secondary text-sm'>WG Endpoint</dt>
          <dd className='m-0 text-sm'>{peer.wg_endpoint || '—'}</dd>
          <dt className='font-semibold text-howm-text-secondary text-sm'>Last seen</dt>
          <dd className='m-0 text-sm'>{formatLastSeen(peer.last_seen, dataUpdatedAt)}</dd>
        </dl>
      </section>

      {/* Access Level */}
      <section className='bg-howm-bg-surface border border-howm-border rounded-xl p-5 mb-4'>
        <h3 className='text-base font-semibold m-0 mb-3 text-howm-text-secondary'>Access Level</h3>
        <TierSelector
          currentTierGroupId={currentTierGroupId}
          onSelect={handleTierSelect}
          disabled={moveMutation.isPending}
        />
        {demotionTarget && (
          <DemotionWarning
            peerName={peer.name}
            targetGroupId={demotionTarget}
            currentCapabilities={currentCaps}
            onConfirm={() => moveMutation.mutate(demotionTarget)}
            onCancel={() => setDemotionTarget(null)}
            isPending={moveMutation.isPending}
          />
        )}
        <div className='mt-4'>
          <GroupChips
            groups={peerGroups}
            allGroups={allGroups}
            onRemove={groupId => removeGroupMutation.mutate(groupId)}
            onAdd={groupId => addGroupMutation.mutate(groupId)}
          />
        </div>
      </section>

      {/* Effective Permissions */}
      <section className='bg-howm-bg-surface border border-howm-border rounded-xl p-5 mb-4'>
        <h3 className='text-base font-semibold m-0 mb-3 text-howm-text-secondary'>Effective Permissions</h3>
        <PermissionGrid permissions={permissions} isLoading={permLoading} />
      </section>

      {/* Deny */}
      <button onClick={() => setShowDenyModal(true)} className='py-2.5 px-5 bg-[rgba(239,68,68,0.12)] border border-[rgba(239,68,68,0.3)] rounded-lg text-howm-error cursor-pointer text-sm font-semibold w-full'>
        🔴 Deny Peer
      </button>

      {showDenyModal && (
        <DenyModal
          peerName={peer.name}
          onConfirm={() => denyMutation.mutate()}
          onCancel={() => setShowDenyModal(false)}
          isPending={denyMutation.isPending}
        />
      )}

      {/* Toasts */}
      {toasts.length > 0 && (
        <div className='fixed bottom-6 left-1/2 -translate-x-1/2 flex flex-col gap-2 z-300'>
          {toasts.map(t => (
            <div key={t.id} className='py-2 px-4 rounded-lg text-sm shadow-[0_4px_12px_rgba(0,0,0,0.5)]' style={{
              background: t.level === 'success' ? '#14532d' : '#7f1d1d',
              color: t.level === 'success' ? '#86efac' : '#fca5a5',
              border: `1px solid ${t.level === 'success' ? '#16a34a' : '#dc2626'}`,
            }}>
              {t.msg}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
