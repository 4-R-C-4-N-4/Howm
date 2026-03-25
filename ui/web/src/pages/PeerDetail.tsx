import { useParams, useNavigate, Link } from 'react-router-dom';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { useState, useCallback, useRef } from 'react';
import { getPeers } from '../api/nodes';
import {
  getPeerGroups, getPeerPermissions, getAccessGroups,
  movePeerToTier, denyPeer, assignPeerToGroup, removePeerFromGroup,
} from '../api/access';
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
      <div style={pageStyle}>
        <Link to="/peers" style={backStyle}>← Back to Peers</Link>
        <p style={mutedStyle}>Peer not found</p>
      </div>
    );
  }

  return (
    <div style={pageStyle}>
      <Link to="/peers" style={backStyle}>← Back to Peers</Link>

      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'flex-start', marginTop: '12px', marginBottom: '8px' }}>
        <div>
          <h1 style={{ margin: 0, fontSize: '1.5rem' }}>{peer.name}</h1>
          <span style={{
            display: 'inline-block', marginTop: '4px',
            fontSize: '0.85rem', padding: '2px 10px', borderRadius: '4px',
            background: badge.bg, color: badge.color,
          }}>
            {badge.label}
          </span>
        </div>
        <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'flex-end', gap: '6px' }}>
          <span style={{ color: online ? '#4ade80' : '#5c6170', fontSize: '0.9rem' }}>
            {online ? '● Online' : '○ Offline'}
          </span>
          <Link
            to={`/app/social.messaging?peer=${encodeURIComponent(peer.wg_pubkey)}`}
            style={{
              background: 'var(--howm-accent, #6c8cff)',
              color: '#fff',
              border: 'none',
              borderRadius: '6px',
              padding: '5px 14px',
              fontSize: '0.85rem',
              fontWeight: 600,
              textDecoration: 'none',
              cursor: 'pointer',
            }}
          >
            Message
          </Link>
        </div>
      </div>

      {/* Identity */}
      <section style={cardStyle}>
        <h3 style={h3Style}>Identity</h3>
        <dl style={dlStyle}>
          <dt style={dtStyle}>Node ID</dt>
          <dd style={ddStyle}>
            <code style={monoStyle}>{peerId!.slice(0, 16)}…{peerId!.slice(-8)}</code>
            <button onClick={() => copyToClipboard(peerId!)} style={copyBtnStyle}>copy</button>
          </dd>
          <dt style={dtStyle}>WG Pubkey</dt>
          <dd style={ddStyle}>
            <code style={monoStyle}>{peer.wg_pubkey}</code>
            <button onClick={() => copyToClipboard(peer.wg_pubkey)} style={copyBtnStyle}>copy</button>
          </dd>
          <dt style={dtStyle}>WG Address</dt>
          <dd style={ddStyle}>{peer.wg_address}</dd>
          <dt style={dtStyle}>WG Endpoint</dt>
          <dd style={ddStyle}>{peer.wg_endpoint || '—'}</dd>
          <dt style={dtStyle}>Last seen</dt>
          <dd style={ddStyle}>{formatLastSeen(peer.last_seen, dataUpdatedAt)}</dd>
        </dl>
      </section>

      {/* Access Level */}
      <section style={cardStyle}>
        <h3 style={h3Style}>Access Level</h3>
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
        <div style={{ marginTop: '16px' }}>
          <GroupChips
            groups={peerGroups}
            allGroups={allGroups}
            onRemove={groupId => removeGroupMutation.mutate(groupId)}
            onAdd={groupId => addGroupMutation.mutate(groupId)}
          />
        </div>
      </section>

      {/* Effective Permissions */}
      <section style={cardStyle}>
        <h3 style={h3Style}>Effective Permissions</h3>
        <PermissionGrid permissions={permissions} isLoading={permLoading} />
      </section>

      {/* Deny */}
      <button onClick={() => setShowDenyModal(true)} style={denyBtnStyle}>
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
const backStyle: React.CSSProperties = {
  color: 'var(--howm-text-muted, #5c6170)', textDecoration: 'none', fontSize: '0.9rem',
};
const h3Style: React.CSSProperties = {
  fontSize: '1rem', fontWeight: 600, margin: '0 0 12px',
  color: 'var(--howm-text-secondary, #8b91a0)',
};
const cardStyle: React.CSSProperties = {
  background: 'var(--howm-bg-surface, #232733)',
  border: '1px solid var(--howm-border, #2e3341)',
  borderRadius: '12px', padding: '20px', marginBottom: '16px',
};
const dlStyle: React.CSSProperties = { display: 'grid', gridTemplateColumns: 'auto 1fr', gap: '6px 16px', margin: 0 };
const dtStyle: React.CSSProperties = { fontWeight: 600, color: 'var(--howm-text-secondary, #8b91a0)', fontSize: '0.85rem' };
const ddStyle: React.CSSProperties = { margin: 0, fontSize: '0.9rem', display: 'flex', alignItems: 'center', gap: '8px' };
const monoStyle: React.CSSProperties = { fontFamily: 'var(--howm-font-mono, monospace)', wordBreak: 'break-all', fontSize: '0.85rem' };
const copyBtnStyle: React.CSSProperties = {
  background: 'none', border: '1px solid var(--howm-border, #2e3341)',
  borderRadius: '3px', color: 'var(--howm-text-muted, #5c6170)',
  cursor: 'pointer', fontSize: '0.7rem', padding: '1px 6px',
};
const mutedStyle: React.CSSProperties = { color: 'var(--howm-text-muted, #5c6170)', fontSize: '0.9rem' };
const denyBtnStyle: React.CSSProperties = {
  padding: '10px 20px', background: 'rgba(248,113,113,0.12)',
  border: '1px solid rgba(248,113,113,0.3)', borderRadius: '8px',
  color: '#f87171', cursor: 'pointer', fontSize: '0.9rem', fontWeight: 600,
  width: '100%',
};
