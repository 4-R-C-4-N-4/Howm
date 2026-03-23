import { useQuery, useQueryClient } from '@tanstack/react-query';
import { getNodeInfo } from '../api/nodes';
import { getApiToken, setApiToken, clearApiToken } from '../api/client';
import { PeerList } from '../components/PeerList';
import { CapabilityList } from '../components/CapabilityList';
import { useState } from 'react';

export function Dashboard() {
  const queryClient = useQueryClient();
  const { data: nodeInfo, isLoading: nodeLoading } = useQuery({
    queryKey: ['node-info'],
    queryFn: getNodeInfo,
  });

  const [tokenInput, setTokenInput] = useState('');
  const hasToken = !!getApiToken();

  const handleSetToken = () => {
    if (tokenInput.trim()) {
      setApiToken(tokenInput.trim());
      setTokenInput('');
      queryClient.invalidateQueries();
    }
  };
  const handleClearToken = () => {
    clearApiToken();
    queryClient.invalidateQueries();
  };

  return (
    <div style={pageStyle}>
      <h1 style={h1Style}>Dashboard</h1>

      {/* API Token */}
      <section style={cardStyle}>
        <h2 style={h2Style}>API Token</h2>
        {hasToken ? (
          <div style={{ display: 'flex', alignItems: 'center', gap: '12px' }}>
            <span style={{ color: 'var(--howm-success, #4ade80)', fontWeight: 600 }}>● Connected</span>
            <span style={mutedStyle}>Token is set</span>
            <button onClick={handleClearToken} style={{ ...btnStyle, marginLeft: 'auto', background: 'rgba(248,113,113,0.15)', color: 'var(--howm-error, #f87171)', border: '1px solid var(--howm-error, #f87171)' }}>
              Clear Token
            </button>
          </div>
        ) : (
          <div>
            <p style={{ color: 'var(--howm-warning, #fbbf24)', margin: '0 0 8px' }}>
              ⚠ No API token set — mutations (invites, peer removal, posting) will be rejected.
            </p>
            <p style={{ ...mutedStyle, fontSize: '0.85em', margin: '0 0 12px' }}>
              Copy the token printed by the daemon on first run (or from the howm.sh startup box).
            </p>
            <div style={{ display: 'flex', gap: '8px' }}>
              <input
                type="password"
                placeholder="Paste API token..."
                value={tokenInput}
                onChange={e => setTokenInput(e.target.value)}
                onKeyDown={e => e.key === 'Enter' && handleSetToken()}
                style={inputStyle}
              />
              <button onClick={handleSetToken} disabled={!tokenInput.trim()} style={accentBtnStyle}>
                Set Token
              </button>
            </div>
          </div>
        )}
      </section>

      {/* Node Info */}
      <section style={cardStyle}>
        <h2 style={h2Style}>Node Info</h2>
        {nodeLoading ? <p style={mutedStyle}>Loading…</p> : nodeInfo && (
          <dl style={dlStyle}>
            <Row label="Node ID" value={nodeInfo.node_id} mono />
            <Row label="Name"    value={nodeInfo.name} />
          </dl>
        )}
      </section>

      {/* Peers */}
      <section style={cardStyle}>
        <PeerList />
      </section>

      {/* Capabilities */}
      <section style={cardStyle}>
        <CapabilityList />
      </section>
    </div>
  );
}

function Row({ label, value, mono }: { label: string; value: string; mono?: boolean }) {
  return (
    <>
      <dt style={dtStyle}>{label}</dt>
      <dd style={{ ...ddStyle, fontFamily: mono ? 'var(--howm-font-mono, monospace)' : 'inherit', wordBreak: 'break-all' }}>
        {value}
      </dd>
    </>
  );
}

const pageStyle: React.CSSProperties = { maxWidth: '800px', margin: '0 auto', padding: '24px' };
const h1Style: React.CSSProperties = { fontSize: 'var(--howm-font-size-2xl, 1.5rem)', marginBottom: '24px', fontWeight: 600 };
const h2Style: React.CSSProperties = { fontSize: 'var(--howm-font-size-xl, 1.25rem)', fontWeight: 600, marginTop: 0, marginBottom: '16px' };
const cardStyle: React.CSSProperties = {
  background: 'var(--howm-bg-surface, #232733)',
  border: '1px solid var(--howm-border, #2e3341)',
  borderRadius: 'var(--howm-radius-lg, 12px)',
  padding: '20px',
  marginBottom: '20px',
};
const dlStyle: React.CSSProperties = { display: 'grid', gridTemplateColumns: 'auto 1fr', gap: '6px 16px', margin: 0 };
const dtStyle: React.CSSProperties = { fontWeight: 600, color: 'var(--howm-text-secondary, #8b91a0)', fontSize: '0.875rem', alignSelf: 'start', paddingTop: '1px' };
const ddStyle: React.CSSProperties = { margin: 0, fontSize: '0.9rem' };
const mutedStyle: React.CSSProperties = { color: 'var(--howm-text-muted, #5c6170)', margin: 0, fontSize: '0.9rem' };
const inputStyle: React.CSSProperties = {
  flex: 1, padding: '6px 10px',
  background: 'var(--howm-bg-secondary, #1a1d27)',
  border: '1px solid var(--howm-border, #2e3341)',
  borderRadius: 'var(--howm-radius-sm, 4px)',
  color: 'var(--howm-text-primary, #e1e4eb)',
  fontSize: '0.9em', fontFamily: 'var(--howm-font-mono, monospace)',
};
const btnStyle: React.CSSProperties = {
  padding: '6px 14px',
  background: 'var(--howm-bg-elevated, #2a2e3d)',
  border: '1px solid var(--howm-border, #2e3341)',
  borderRadius: 'var(--howm-radius-sm, 4px)',
  color: 'var(--howm-text-primary, #e1e4eb)',
  cursor: 'pointer', fontSize: '0.9em',
};
const accentBtnStyle: React.CSSProperties = {
  padding: '6px 14px',
  background: 'var(--howm-accent, #6c8cff)',
  border: 'none',
  borderRadius: 'var(--howm-radius-sm, 4px)',
  color: '#fff', cursor: 'pointer', fontSize: '0.9em',
};
