import { useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { getNetworkStatus } from '../api/network';
import { NetworkStatus } from '../components/NetworkStatus';
import { ConnectionInfo } from '../components/ConnectionInfo';
import { InviteManager } from '../components/InviteManager';
import { RelayConfig } from '../components/RelayConfig';

export function Connection() {
  const [infoOpen, setInfoOpen] = useState(false);

  const { data: status, isLoading } = useQuery({
    queryKey: ['network-status'],
    queryFn: getNetworkStatus,
    refetchInterval: 15000,
  });

  if (isLoading || !status) {
    return (
      <div style={pageStyle}>
        <h1 style={h1Style}>Connection</h1>
        <p style={mutedStyle}>Loading network status…</p>
      </div>
    );
  }

  return (
    <div style={pageStyle}>
      {/* Header with info button */}
      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '24px' }}>
        <h1 style={{ ...h1Style, marginBottom: 0 }}>Connection</h1>
        <button onClick={() => setInfoOpen(true)} style={infoBtnStyle} title="How does this work?">
          ⓘ Info
        </button>
      </div>

      {/* Network Status */}
      <NetworkStatus status={status} />

      {/* Invites */}
      <InviteManager reachability={status.reachability} />

      {/* Relay */}
      <RelayConfig relay={status.relay} />

      {/* Info drawer */}
      <ConnectionInfo open={infoOpen} onClose={() => setInfoOpen(false)} status={status} />
    </div>
  );
}

// ── Styles ───────────────────────────────────────────────────────────────────

const pageStyle: React.CSSProperties = {
  maxWidth: '800px', margin: '0 auto', padding: '24px',
};
const h1Style: React.CSSProperties = {
  fontSize: 'var(--howm-font-size-2xl, 1.5rem)',
  marginBottom: '24px', fontWeight: 600,
};
const mutedStyle: React.CSSProperties = {
  color: 'var(--howm-text-muted, #5c6170)',
  margin: 0, fontSize: '0.9rem',
};
const infoBtnStyle: React.CSSProperties = {
  padding: '6px 14px',
  background: 'rgba(108,140,255,0.1)',
  border: '1px solid var(--howm-accent, #6c8cff)',
  borderRadius: 'var(--howm-radius-sm, 4px)',
  color: 'var(--howm-accent, #6c8cff)',
  cursor: 'pointer', fontSize: '0.875rem',
  fontWeight: 600,
};
