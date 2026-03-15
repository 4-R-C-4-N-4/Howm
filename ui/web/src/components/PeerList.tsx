import { useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { getPeers, addPeer, removePeer, generateInvite, redeemInvite } from '../api/nodes';
import type { Peer } from '../api/nodes';

export function PeerList() {
  const queryClient = useQueryClient();
  const { data: peers = [], isLoading } = useQuery({
    queryKey: ['peers'],
    queryFn: getPeers,
    refetchInterval: 30000,
  });

  const [showAddForm, setShowAddForm] = useState(false);
  const [showRedeemForm, setShowRedeemForm] = useState(false);
  const [address, setAddress] = useState('');
  const [port, setPort] = useState('7000');
  const [authKey, setAuthKey] = useState('');
  const [inviteCode, setInviteCode] = useState('');
  const [generatedInvite, setGeneratedInvite] = useState<string | null>(null);

  const addMutation = useMutation({
    mutationFn: () => addPeer(address, parseInt(port), authKey || undefined),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['peers'] });
      setShowAddForm(false);
      setAddress(''); setPort('7000'); setAuthKey('');
    },
  });

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

  return (
    <div>
      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '12px' }}>
        <h3 style={{ margin: 0 }}>Peers ({peers.length})</h3>
        <div style={{ display: 'flex', gap: '8px' }}>
          <button onClick={() => inviteMutation.mutate()} style={btnStyle}>
            Generate Invite
          </button>
          <button onClick={() => setShowRedeemForm(!showRedeemForm)} style={btnStyle}>
            Redeem Invite
          </button>
          <button onClick={() => setShowAddForm(!showAddForm)} style={btnStyle}>
            Add Manually
          </button>
        </div>
      </div>

      {generatedInvite && (
        <div style={{ background: '#f0f9ff', border: '1px solid #0ea5e9', borderRadius: '6px', padding: '12px', marginBottom: '12px' }}>
          <strong>Invite Link:</strong>
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
            style={inputStyle}
          />
          <button onClick={() => redeemMutation.mutate()} disabled={!inviteCode.trim()} style={btnStyle}>
            {redeemMutation.isPending ? 'Redeeming...' : 'Redeem'}
          </button>
          {redeemMutation.isError && <span style={{ color: 'red' }}> Failed</span>}
        </div>
      )}

      {showAddForm && (
        <div style={formStyle}>
          <input placeholder="Address" value={address} onChange={e => setAddress(e.target.value)} style={inputStyle} />
          <input placeholder="Port" value={port} onChange={e => setPort(e.target.value)} style={{ ...inputStyle, width: '80px' }} />
          <input placeholder="Auth key (optional)" value={authKey} onChange={e => setAuthKey(e.target.value)} style={inputStyle} />
          <button onClick={() => addMutation.mutate()} disabled={!address.trim()} style={btnStyle}>
            {addMutation.isPending ? 'Adding...' : 'Add Peer'}
          </button>
          {addMutation.isError && <span style={{ color: 'red' }}> Failed</span>}
        </div>
      )}

      {isLoading ? (
        <p>Loading peers...</p>
      ) : peers.length === 0 ? (
        <p style={{ color: '#888' }}>No peers yet. Add one above.</p>
      ) : (
        <ul style={{ listStyle: 'none', padding: 0 }}>
          {peers.map((peer: Peer) => (
            <li key={peer.node_id} style={{
              display: 'flex', justifyContent: 'space-between', alignItems: 'center',
              padding: '8px 12px', border: '1px solid #eee', borderRadius: '6px', marginBottom: '6px',
            }}>
              <div>
                <strong>{peer.name}</strong>
                <span style={{ color: '#888', marginLeft: '8px', fontSize: '0.85em' }}>
                  {peer.address}:{peer.port}
                </span>
              </div>
              <button
                onClick={() => removeMutation.mutate(peer.node_id)}
                style={{ background: '#fee2e2', border: 'none', borderRadius: '4px', padding: '4px 10px', cursor: 'pointer' }}
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
