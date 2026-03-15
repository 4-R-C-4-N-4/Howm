import { useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { getPeers, removePeer, generateInvite, redeemInvite } from '../api/nodes';
import type { Peer } from '../api/nodes';

export function PeerList() {
  const queryClient = useQueryClient();
  const { data: peers = [], isLoading } = useQuery({
    queryKey: ['peers'],
    queryFn: getPeers,
    refetchInterval: 30000,
  });

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

  const formatLastSeen = (ts: number) => {
    if (!ts) return 'never';
    const delta = Math.floor(Date.now() / 1000 - ts);
    if (delta < 60) return 'just now';
    if (delta < 3600) return `${Math.floor(delta / 60)}m ago`;
    if (delta < 86400) return `${Math.floor(delta / 3600)}h ago`;
    return `${Math.floor(delta / 86400)}d ago`;
  };

  return (
    <div>
      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '12px' }}>
        <h3 style={{ margin: 0 }}>Peers ({peers.length})</h3>
        <div style={{ display: 'flex', gap: '8px' }}>
          <button onClick={() => inviteMutation.mutate()} style={btnStyle}>
            {inviteMutation.isPending ? 'Generating...' : 'Generate Invite'}
          </button>
          <button onClick={() => setShowRedeemForm(!showRedeemForm)} style={btnStyle}>
            Redeem Invite
          </button>
        </div>
      </div>

      {inviteMutation.isError && (
        <div style={{ color: '#ef4444', marginBottom: '8px', fontSize: '0.9em' }}>
          Failed to generate invite. Is the API token set?
        </div>
      )}

      {generatedInvite && (
        <div style={{ background: '#f0f9ff', border: '1px solid #0ea5e9', borderRadius: '6px', padding: '12px', marginBottom: '12px' }}>
          <strong>Invite Code:</strong>
          <div style={{ wordBreak: 'break-all', marginTop: '4px', fontFamily: 'monospace', fontSize: '0.85em' }}>
            {generatedInvite}
          </div>
          <button onClick={() => navigator.clipboard?.writeText(generatedInvite)} style={{ ...btnStyle, marginTop: '8px' }}>
            Copy
          </button>
          <button onClick={() => setGeneratedInvite(null)} style={{ ...btnStyle, marginLeft: '8px' }}>
            Dismiss
          </button>
        </div>
      )}

      {showRedeemForm && (
        <div style={formStyle}>
          <input
            placeholder="howm://invite/..."
            value={inviteCode}
            onChange={e => setInviteCode(e.target.value)}
            style={{ ...inputStyle, flex: 1 }}
          />
          <button onClick={() => redeemMutation.mutate()} disabled={!inviteCode.trim()} style={btnStyle}>
            {redeemMutation.isPending ? 'Redeeming...' : 'Redeem'}
          </button>
          {redeemMutation.isError && <span style={{ color: 'red', fontSize: '0.9em' }}> Failed — check code and token</span>}
        </div>
      )}

      {isLoading ? (
        <p>Loading peers...</p>
      ) : peers.length === 0 ? (
        <p style={{ color: '#888' }}>No peers yet. Generate an invite or redeem one from a friend.</p>
      ) : (
        <ul style={{ listStyle: 'none', padding: 0 }}>
          {peers.map((peer: Peer) => (
            <li key={peer.node_id} style={{
              display: 'flex', justifyContent: 'space-between', alignItems: 'center',
              padding: '10px 12px', border: '1px solid #eee', borderRadius: '6px', marginBottom: '6px',
            }}>
              <div>
                <strong>{peer.name}</strong>
                <span style={{ color: '#888', marginLeft: '10px', fontSize: '0.85em', fontFamily: 'monospace' }}>
                  {peer.wg_address}
                </span>
                <span style={{ color: '#aaa', marginLeft: '8px', fontSize: '0.8em' }}>
                  {formatLastSeen(peer.last_seen)}
                </span>
              </div>
              <button
                onClick={() => removeMutation.mutate(peer.node_id)}
                style={{ background: '#fee2e2', border: 'none', borderRadius: '4px', padding: '4px 10px', cursor: 'pointer', fontSize: '0.85em' }}
              >
                Remove
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

const btnStyle: React.CSSProperties = {
  padding: '6px 14px', background: '#f3f4f6', border: '1px solid #ddd',
  borderRadius: '6px', cursor: 'pointer', fontSize: '0.9em',
};
const formStyle: React.CSSProperties = {
  display: 'flex', gap: '8px', alignItems: 'center', marginBottom: '12px',
  padding: '10px', background: '#f9fafb', borderRadius: '6px', flexWrap: 'wrap',
};
const inputStyle: React.CSSProperties = {
  padding: '6px 10px', border: '1px solid #ddd', borderRadius: '6px', fontSize: '0.9em',
};
