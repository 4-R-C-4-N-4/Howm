import { useQuery } from '@tanstack/react-query';
import { getNodeInfo, getTailnet, getAuthKeys, addAuthKey, removeAuthKey } from '../api/nodes';
import { PeerList } from '../components/PeerList';
import { CapabilityList } from '../components/CapabilityList';
import { useState } from 'react';
import { useMutation, useQueryClient } from '@tanstack/react-query';

export function Dashboard() {
  const { data: nodeInfo, isLoading: nodeLoading } = useQuery({
    queryKey: ['node-info'],
    queryFn: getNodeInfo,
  });
  const { data: tailnet } = useQuery({
    queryKey: ['tailnet'],
    queryFn: getTailnet,
  });
  const { data: authKeys = [] } = useQuery({
    queryKey: ['auth-keys'],
    queryFn: getAuthKeys,
  });

  const queryClient = useQueryClient();
  const [newKey, setNewKey] = useState('');

  const addKeyMutation = useMutation({
    mutationFn: addAuthKey,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['auth-keys'] });
      setNewKey('');
    },
  });

  const removeKeyMutation = useMutation({
    mutationFn: removeAuthKey,
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['auth-keys'] }),
  });

  return (
    <div style={{ maxWidth: '800px', margin: '0 auto', padding: '24px' }}>
      <h1 style={{ marginBottom: '24px' }}>Dashboard</h1>

      {/* Node Info Card */}
      <div style={cardStyle}>
        <h2 style={{ marginTop: 0, marginBottom: '16px' }}>Node Info</h2>
        {nodeLoading ? <p>Loading...</p> : nodeInfo && (
          <dl style={{ display: 'grid', gridTemplateColumns: 'auto 1fr', gap: '6px 16px', margin: 0 }}>
            <dt style={dtStyle}>Node ID</dt>
            <dd style={ddStyle}>{nodeInfo.node_id}</dd>
            <dt style={dtStyle}>Name</dt>
            <dd style={ddStyle}>{nodeInfo.name}</dd>
            <dt style={dtStyle}>Tailnet IP</dt>
            <dd style={ddStyle}>{tailnet?.tailnet_ip ?? 'Not connected'}</dd>
            <dt style={dtStyle}>Tailnet Status</dt>
            <dd style={ddStyle}>
              <span style={{
                color: tailnet?.status === 'connected' ? '#22c55e' : '#f59e0b',
              }}>
                {tailnet?.status ?? 'unknown'}
              </span>
            </dd>
            {tailnet?.coordination_url && (
              <>
                <dt style={dtStyle}>Coordination URL</dt>
                <dd style={ddStyle}>{tailnet.coordination_url}</dd>
              </>
            )}
          </dl>
        )}
      </div>

      {/* Peer List */}
      <div style={cardStyle}>
        <PeerList />
      </div>

      {/* Auth Keys */}
      <div style={cardStyle}>
        <h3 style={{ marginTop: 0 }}>Auth Keys</h3>
        {authKeys.length === 0 ? (
          <p style={{ color: '#888' }}>No auth keys configured.</p>
        ) : (
          <ul style={{ listStyle: 'none', padding: 0 }}>
            {authKeys.map(k => (
              <li key={k.prefix} style={{ display: 'flex', justifyContent: 'space-between', marginBottom: '6px', padding: '6px 10px', background: '#f9fafb', borderRadius: '4px' }}>
                <code>{k.prefix}...</code>
                <button onClick={() => removeKeyMutation.mutate(k.prefix)} style={{ background: '#fee2e2', border: 'none', borderRadius: '4px', padding: '2px 8px', cursor: 'pointer' }}>
                  Remove
                </button>
              </li>
            ))}
          </ul>
        )}
        <div style={{ display: 'flex', gap: '8px', marginTop: '12px' }}>
          <input
            placeholder="psk-..."
            value={newKey}
            onChange={e => setNewKey(e.target.value)}
            style={{ padding: '6px 10px', border: '1px solid #ddd', borderRadius: '6px', fontSize: '0.9em', flex: 1 }}
          />
          <button
            onClick={() => newKey.trim() && addKeyMutation.mutate(newKey.trim())}
            disabled={!newKey.trim()}
            style={{ padding: '6px 14px', background: '#4f46e5', color: '#fff', border: 'none', borderRadius: '6px', cursor: 'pointer' }}
          >
            Add Key
          </button>
        </div>
      </div>

      {/* Capability List */}
      <div style={cardStyle}>
        <CapabilityList />
      </div>
    </div>
  );
}

const cardStyle: React.CSSProperties = {
  background: '#fff', border: '1px solid #e5e7eb', borderRadius: '12px',
  padding: '20px', marginBottom: '20px', boxShadow: '0 1px 3px rgba(0,0,0,0.06)',
};
const dtStyle: React.CSSProperties = { fontWeight: 600, color: '#6b7280', fontSize: '0.9em' };
const ddStyle: React.CSSProperties = { margin: 0, fontFamily: 'monospace', fontSize: '0.9em' };
