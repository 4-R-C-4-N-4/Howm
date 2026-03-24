import { Link } from 'react-router-dom';
import { useQuery } from '@tanstack/react-query';
import { getCapabilities } from '../api/capabilities';
import type { Capability } from '../api/capabilities';

export function CapabilityList() {
  const { data: capabilities = [], isLoading } = useQuery({
    queryKey: ['capabilities'],
    queryFn: getCapabilities,
    refetchInterval: 30000,
  });

  const statusColor = (status: Capability['status']) => {
    if (status === 'Running') return 'var(--howm-success, #4ade80)';
    if (status === 'Stopped') return 'var(--howm-warning, #fbbf24)';
    return 'var(--howm-error, #f87171)';
  };

  const statusLabel = (status: Capability['status']) => {
    if (typeof status === 'string') return status;
    if (status && typeof status === 'object' && 'Error' in status) return `Error: ${status.Error}`;
    return 'Unknown';
  };

  if (isLoading) return <p style={mutedStyle}>Loading capabilities…</p>;

  return (
    <div>
      <h3 style={{ margin: '0 0 12px' }}>Capabilities ({capabilities.length})</h3>
      {capabilities.length === 0 ? (
        <p style={mutedStyle}>No capabilities installed.</p>
      ) : (
        <ul style={{ listStyle: 'none', padding: 0, margin: 0 }}>
          {capabilities.map((cap: Capability) => (
            <li key={cap.name} style={rowStyle}>
              <div style={{ display: 'flex', alignItems: 'center', gap: '8px', minWidth: 0 }}>
                <strong style={{ whiteSpace: 'nowrap' }}>{cap.name}</strong>
                <span style={mutedStyle}>v{cap.version}</span>
                <span style={{ ...mutedStyle, fontFamily: 'var(--howm-font-mono, monospace)', fontSize: '0.8em' }}>:{cap.port}</span>
                {/* Task 4: link to capability UI page if available */}
                {cap.ui && (
                  <Link
                    to={`/app/${cap.name}`}
                    style={{
                      fontSize: '0.8em',
                      padding: '2px 8px',
                      background: 'var(--howm-accent-dim, rgba(108,140,255,0.15))',
                      color: 'var(--howm-accent, #6c8cff)',
                      borderRadius: 'var(--howm-radius-sm, 4px)',
                      textDecoration: 'none',
                      whiteSpace: 'nowrap',
                    }}
                  >
                    Open {cap.ui.label} →
                  </Link>
                )}
              </div>
              <span style={{
                background: `${statusColor(cap.status)}1a`,
                color: statusColor(cap.status),
                border: `1px solid ${statusColor(cap.status)}4d`,
                borderRadius: '12px', padding: '2px 10px', fontSize: '0.8em', whiteSpace: 'nowrap',
              }}>
                {statusLabel(cap.status)}
              </span>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

const mutedStyle: React.CSSProperties = { color: 'var(--howm-text-muted, #5c6170)', margin: 0, fontSize: '0.875rem' };
const rowStyle: React.CSSProperties = {
  padding: '10px 12px',
  border: '1px solid var(--howm-border, #2e3341)',
  borderRadius: 'var(--howm-radius-sm, 4px)',
  marginBottom: '6px',
  display: 'flex', justifyContent: 'space-between', alignItems: 'center', gap: '12px',
  background: 'var(--howm-bg-secondary, #1a1d27)',
};
