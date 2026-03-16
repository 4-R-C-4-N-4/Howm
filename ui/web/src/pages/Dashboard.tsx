import { useQuery } from '@tanstack/react-query';
import { getNodeInfo, getWgStatus } from '../api/nodes';
import { getApiToken, setApiToken, clearApiToken } from '../api/client';
import { PeerList } from '../components/PeerList';
import { CapabilityList } from '../components/CapabilityList';
import { OpenInviteSection } from '../components/OpenInviteSection';
import { useState } from 'react';

export function Dashboard() {
  const { data: nodeInfo, isLoading: nodeLoading } = useQuery({
    queryKey: ['node-info'],
    queryFn: getNodeInfo,
  });
  const { data: wgStatus } = useQuery({
    queryKey: ['wg-status'],
    queryFn: getWgStatus,
    refetchInterval: 15000,
  });

  const [tokenInput, setTokenInput] = useState('');
  const hasToken = !!getApiToken();

  const handleSetToken = () => {
    if (tokenInput.trim()) {
      setApiToken(tokenInput.trim());
      setTokenInput('');
      window.location.reload();
    }
  };

  return (
    <div style={{ maxWidth: '800px', margin: '0 auto', padding: '24px' }}>
      <h1 style={{ marginBottom: '24px' }}>Dashboard</h1>

      {/* API Token */}
      <div style={cardStyle}>
        <h2 style={{ marginTop: 0, marginBottom: '12px' }}>API Token</h2>
        {hasToken ? (
          <div style={{ display: 'flex', alignItems: 'center', gap: '12px' }}>
            <span style={{ color: '#22c55e', fontWeight: 600 }}>● Connected</span>
            <span style={{ color: '#888', fontSize: '0.85em' }}>Token is set</span>
            <button onClick={() => { clearApiToken(); window.location.reload(); }}
              style={{ marginLeft: 'auto', padding: '4px 12px', background: '#fee2e2', border: 'none', borderRadius: '6px', cursor: 'pointer' }}>
              Clear Token
            </button>
          </div>
        ) : (
          <div>
            <p style={{ color: '#f59e0b', marginTop: 0 }}>
              ⚠ No API token set — mutations (invites, peer removal, posting) will be rejected.
            </p>
            <p style={{ color: '#888', fontSize: '0.85em', margin: '8px 0' }}>
              Paste the token printed by the daemon on first run.
            </p>
            <div style={{ display: 'flex', gap: '8px' }}>
              <input
                type="password"
                placeholder="Paste API token..."
                value={tokenInput}
                onChange={e => setTokenInput(e.target.value)}
                onKeyDown={e => e.key === 'Enter' && handleSetToken()}
                style={{ padding: '6px 10px', border: '1px solid #ddd', borderRadius: '6px', fontSize: '0.9em', flex: 1, fontFamily: 'monospace' }}
              />
              <button onClick={handleSetToken} disabled={!tokenInput.trim()}
                style={{ padding: '6px 14px', background: '#4f46e5', color: '#fff', border: 'none', borderRadius: '6px', cursor: 'pointer' }}>
                Set Token
              </button>
            </div>
          </div>
        )}
      </div>

      {/* Node Info Card */}
      <div style={cardStyle}>
        <h2 style={{ marginTop: 0, marginBottom: '16px' }}>Node Info</h2>
        {nodeLoading ? <p>Loading...</p> : nodeInfo && (
          <dl style={{ display: 'grid', gridTemplateColumns: 'auto 1fr', gap: '6px 16px', margin: 0 }}>
            <dt style={dtStyle}>Node ID</dt>
            <dd style={ddStyle}>{nodeInfo.node_id}</dd>
            <dt style={dtStyle}>Name</dt>
            <dd style={ddStyle}>{nodeInfo.name}</dd>
          </dl>
        )}
      </div>

      {/* WireGuard Status Card */}
      <div style={cardStyle}>
        <h2 style={{ marginTop: 0, marginBottom: '16px' }}>WireGuard</h2>
        {wgStatus ? (
          <dl style={{ display: 'grid', gridTemplateColumns: 'auto 1fr', gap: '6px 16px', margin: 0 }}>
            <dt style={dtStyle}>Status</dt>
            <dd style={ddStyle}>
              <span style={{ color: wgStatus.status === 'connected' ? '#22c55e' : '#f59e0b' }}>
                {wgStatus.status}
              </span>
            </dd>
            {wgStatus.public_key && (
              <>
                <dt style={dtStyle}>Public Key</dt>
                <dd style={{ ...ddStyle, wordBreak: 'break-all' }}>{wgStatus.public_key}</dd>
              </>
            )}
            {wgStatus.address && (
              <>
                <dt style={dtStyle}>WG Address</dt>
                <dd style={ddStyle}>{wgStatus.address}</dd>
              </>
            )}
            {wgStatus.endpoint && (
              <>
                <dt style={dtStyle}>Endpoint</dt>
                <dd style={ddStyle}>{wgStatus.endpoint}</dd>
              </>
            )}
            {wgStatus.listen_port && (
              <>
                <dt style={dtStyle}>Listen Port</dt>
                <dd style={ddStyle}>{wgStatus.listen_port}</dd>
              </>
            )}
            {wgStatus.active_tunnels != null && (
              <>
                <dt style={dtStyle}>Active Tunnels</dt>
                <dd style={ddStyle}>{wgStatus.active_tunnels}</dd>
              </>
            )}
          </dl>
        ) : (
          <p style={{ color: '#888' }}>Loading WireGuard status...</p>
        )}
      </div>

      {/* Open Invite */}
      <div style={cardStyle}>
        <OpenInviteSection />
      </div>

      {/* Peer List */}
      <div style={cardStyle}>
        <PeerList />
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
