import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { getPeers, removePeer, updatePeerTrust } from '../api/nodes';
import type { Peer, TrustLevel } from '../api/nodes';
import { Link } from 'react-router-dom';

function formatLastSeen(ts: number, now: number) {
  if (!ts) return 'never';
  const delta = Math.floor(now / 1000 - ts);
  if (delta < 60) return 'just now';
  if (delta < 3600) return `${Math.floor(delta / 60)}m ago`;
  if (delta < 86400) return `${Math.floor(delta / 3600)}h ago`;
  return `${Math.floor(delta / 86400)}d ago`;
}

const trustBadge: Record<TrustLevel, { label: string; color: string; bg: string }> = {
  friend:     { label: 'Friend',     color: 'var(--howm-success, #4ade80)',  bg: 'rgba(74,222,128,0.12)'  },
  public:     { label: 'Public',     color: 'var(--howm-warning, #fbbf24)',  bg: 'rgba(251,191,36,0.12)'  },
  restricted: { label: 'Restricted', color: 'var(--howm-error, #f87171)',    bg: 'rgba(248,113,113,0.12)' },
};

export function PeerList() {
  const queryClient = useQueryClient();
  const { data: peers = [], isLoading, dataUpdatedAt } = useQuery({
    queryKey: ['peers'],
    queryFn: getPeers,
    refetchInterval: 30000,
  });
  const now = dataUpdatedAt;

  const removeMutation = useMutation({
    mutationFn: removePeer,
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['peers'] }),
  });

  const trustMutation = useMutation({
    mutationFn: ({ node_id, trust }: { node_id: string; trust: TrustLevel }) =>
      updatePeerTrust(node_id, trust),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['peers'] }),
  });

  return (
    <div>
      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '12px' }}>
        <h3 style={{ margin: 0 }}>Peers ({peers.length})</h3>
      </div>

      {isLoading ? (
        <p style={mutedStyle}>Loading peers…</p>
      ) : peers.length === 0 ? (
        <p style={mutedStyle}>
          No peers yet. Go to{' '}
          <Link to="/connection" style={linkStyle}>Connection</Link>
          {' '}to create or redeem an invite.
        </p>
      ) : (
        <ul style={{ listStyle: 'none', padding: 0, margin: 0 }}>
          {peers.map((peer: Peer) => {
            const badge = trustBadge[peer.trust || 'friend'];
            return (
              <li key={peer.node_id} style={peerRowStyle}>
                <div>
                  <strong>{peer.name}</strong>
                  <span style={{ marginLeft: '8px', fontSize: '0.75em', padding: '2px 7px', borderRadius: '4px', background: badge.bg, color: badge.color }}>
                    {badge.label}
                  </span>
                  <span style={{ color: 'var(--howm-text-muted, #5c6170)', marginLeft: '10px', fontSize: '0.85em', fontFamily: 'var(--howm-font-mono, monospace)' }}>
                    {peer.wg_address}
                  </span>
                  <span style={{ color: 'var(--howm-text-muted, #5c6170)', marginLeft: '8px', fontSize: '0.8em' }}>
                    {formatLastSeen(peer.last_seen, now)}
                  </span>
                </div>
                <div style={{ display: 'flex', gap: '4px' }}>
                  {peer.trust === 'public' && (
                    <button onClick={() => trustMutation.mutate({ node_id: peer.node_id, trust: 'friend' })} style={trustBtnStyle('success')}>
                      Promote
                    </button>
                  )}
                  {peer.trust === 'friend' && (
                    <button onClick={() => trustMutation.mutate({ node_id: peer.node_id, trust: 'restricted' })} style={trustBtnStyle('warning')}>
                      Restrict
                    </button>
                  )}
                  {peer.trust === 'restricted' && (
                    <button onClick={() => trustMutation.mutate({ node_id: peer.node_id, trust: 'friend' })} style={trustBtnStyle('success')}>
                      Restore
                    </button>
                  )}
                  <button onClick={() => removeMutation.mutate(peer.node_id)} style={trustBtnStyle('error')}>
                    Remove
                  </button>
                </div>
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}

const trustBtnStyle = (variant: 'success' | 'warning' | 'error'): React.CSSProperties => {
  const colors = {
    success: { bg: 'rgba(74,222,128,0.12)',  color: 'var(--howm-success, #4ade80)',  border: 'rgba(74,222,128,0.3)'  },
    warning: { bg: 'rgba(251,191,36,0.12)',  color: 'var(--howm-warning, #fbbf24)',  border: 'rgba(251,191,36,0.3)'  },
    error:   { bg: 'rgba(248,113,113,0.12)', color: 'var(--howm-error, #f87171)',    border: 'rgba(248,113,113,0.3)' },
  }[variant];
  return { background: colors.bg, color: colors.color, border: `1px solid ${colors.border}`, borderRadius: '4px', padding: '3px 8px', cursor: 'pointer', fontSize: '0.8em' };
};

const mutedStyle: React.CSSProperties = { color: 'var(--howm-text-muted, #5c6170)', margin: 0 };
const linkStyle: React.CSSProperties = { color: 'var(--howm-accent, #6c8cff)', textDecoration: 'none' };
const peerRowStyle: React.CSSProperties = {
  display: 'flex', justifyContent: 'space-between', alignItems: 'center',
  padding: '10px 12px',
  border: '1px solid var(--howm-border, #2e3341)',
  borderRadius: 'var(--howm-radius-sm, 4px)', marginBottom: '6px',
  background: 'var(--howm-bg-secondary, #1a1d27)',
};
