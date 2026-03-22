import { useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { getPeers, removePeer, generateInvite, redeemInvite, updatePeerTrust } from '../api/nodes';
import type { Peer, TrustLevel } from '../api/nodes';

function formatLastSeen(ts: number, now: number) {
  if (!ts) return 'never';
  const delta = Math.floor(now / 1000 - ts);
  if (delta < 60) return 'just now';
  if (delta < 3600) return `${Math.floor(delta / 60)}m ago`;
  if (delta < 86400) return `${Math.floor(delta / 3600)}h ago`;
  return `${Math.floor(delta / 86400)}d ago`;
}

function extractErrorMessage(err: unknown): string {
  if (err && typeof err === 'object' && 'response' in err) {
    const res = (err as { response?: { data?: { error?: string; message?: string } } }).response;
    if (res?.data?.error) return res.data.error;
    if (res?.data?.message) return res.data.message;
  }
  if (err instanceof Error) return err.message;
  return String(err);
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

  const [showRedeemForm, setShowRedeemForm] = useState(false);
  const [inviteCode, setInviteCode] = useState('');
  const [generatedInvite, setGeneratedInvite] = useState<string | null>(null);

  const removeMutation = useMutation({
    mutationFn: removePeer,
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['peers'] }),
  });

  const inviteMutation = useMutation({
    mutationFn: () => generateInvite(),
    onSuccess: (data) => setGeneratedInvite(data.invite_code),
  });

  const redeemMutation = useMutation({
    mutationFn: () => redeemInvite(inviteCode),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['peers'] });
      setShowRedeemForm(false);
      setInviteCode('');
    },
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
        <div style={{ display: 'flex', gap: '8px' }}>
          <button onClick={() => inviteMutation.mutate()} style={btnStyle}>
            {inviteMutation.isPending ? 'Generating…' : 'Generate Invite'}
          </button>
          <button onClick={() => setShowRedeemForm(!showRedeemForm)} style={btnStyle}>
            Redeem Invite
          </button>
        </div>
      </div>

      {/* Task 2: show real error messages */}
      {inviteMutation.isError && (
        <div style={errorStyle}>
          {extractErrorMessage(inviteMutation.error)}
        </div>
      )}

      {generatedInvite && (
        <div style={inviteBoxStyle}>
          <strong style={{ fontSize: '0.875rem' }}>Invite Code:</strong>
          <div style={{ wordBreak: 'break-all', marginTop: '6px', fontFamily: 'var(--howm-font-mono, monospace)', fontSize: '0.8em', color: 'var(--howm-text-primary, #e1e4eb)' }}>
            {generatedInvite}
          </div>
          <div style={{ display: 'flex', gap: '8px', marginTop: '10px' }}>
            <button onClick={() => navigator.clipboard?.writeText(generatedInvite)} style={btnStyle}>Copy</button>
            <button onClick={() => setGeneratedInvite(null)} style={btnStyle}>Dismiss</button>
          </div>
        </div>
      )}

      {showRedeemForm && (
        <div style={formStyle}>
          <input
            placeholder="howm://invite/... or howm://open/..."
            value={inviteCode}
            onChange={e => setInviteCode(e.target.value)}
            style={{ ...inputStyle, flex: 1 }}
          />
          <button onClick={() => redeemMutation.mutate()} disabled={!inviteCode.trim()} style={accentBtnStyle}>
            {redeemMutation.isPending ? 'Redeeming…' : 'Redeem'}
          </button>
          {redeemMutation.isError && (
            <span style={{ color: 'var(--howm-error, #f87171)', fontSize: '0.875em' }}>
              {extractErrorMessage(redeemMutation.error)}
            </span>
          )}
        </div>
      )}

      {isLoading ? (
        <p style={mutedStyle}>Loading peers…</p>
      ) : peers.length === 0 ? (
        <p style={mutedStyle}>No peers yet. Generate an invite or redeem one from a friend.</p>
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

const btnStyle: React.CSSProperties = {
  padding: '6px 14px',
  background: 'var(--howm-bg-elevated, #2a2e3d)',
  border: '1px solid var(--howm-border, #2e3341)',
  borderRadius: 'var(--howm-radius-sm, 4px)',
  color: 'var(--howm-text-primary, #e1e4eb)',
  cursor: 'pointer', fontSize: '0.875em',
};
const accentBtnStyle: React.CSSProperties = {
  padding: '6px 14px',
  background: 'var(--howm-accent, #6c8cff)',
  border: 'none',
  borderRadius: 'var(--howm-radius-sm, 4px)',
  color: '#fff', cursor: 'pointer', fontSize: '0.875em',
};
const formStyle: React.CSSProperties = {
  display: 'flex', gap: '8px', alignItems: 'center', marginBottom: '12px',
  padding: '10px 12px', background: 'var(--howm-bg-secondary, #1a1d27)',
  border: '1px solid var(--howm-border, #2e3341)',
  borderRadius: 'var(--howm-radius-sm, 4px)', flexWrap: 'wrap',
};
const inputStyle: React.CSSProperties = {
  padding: '6px 10px',
  background: 'var(--howm-bg-primary, #0f1117)',
  border: '1px solid var(--howm-border, #2e3341)',
  borderRadius: 'var(--howm-radius-sm, 4px)',
  color: 'var(--howm-text-primary, #e1e4eb)',
  fontSize: '0.875em',
};
const inviteBoxStyle: React.CSSProperties = {
  background: 'var(--howm-accent-dim, rgba(108,140,255,0.1))',
  border: '1px solid var(--howm-accent, #6c8cff)',
  borderRadius: 'var(--howm-radius-sm, 4px)',
  padding: '12px', marginBottom: '12px',
};
const errorStyle: React.CSSProperties = {
  background: 'rgba(248,113,113,0.1)',
  border: '1px solid var(--howm-error, #f87171)',
  borderRadius: 'var(--howm-radius-sm, 4px)',
  padding: '8px 12px', marginBottom: '10px',
  fontSize: '0.875em', color: 'var(--howm-error, #f87171)',
};
const mutedStyle: React.CSSProperties = { color: 'var(--howm-text-muted, #5c6170)', margin: 0 };
const peerRowStyle: React.CSSProperties = {
  display: 'flex', justifyContent: 'space-between', alignItems: 'center',
  padding: '10px 12px',
  border: '1px solid var(--howm-border, #2e3341)',
  borderRadius: 'var(--howm-radius-sm, 4px)', marginBottom: '6px',
  background: 'var(--howm-bg-secondary, #1a1d27)',
};
