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
    if (status === 'Running') return '#22c55e';
    if (status === 'Stopped') return '#f59e0b';
    return '#ef4444';
  };

  const statusLabel = (status: Capability['status']) => {
    if (typeof status === 'string') return status;
    if (status && typeof status === 'object' && 'Error' in status) return `Error: ${status.Error}`;
    return 'Unknown';
  };

  if (isLoading) return <p>Loading capabilities...</p>;

  return (
    <div>
      <h3>Capabilities ({capabilities.length})</h3>
      {capabilities.length === 0 ? (
        <p style={{ color: '#888' }}>No capabilities installed.</p>
      ) : (
        <ul style={{ listStyle: 'none', padding: 0 }}>
          {capabilities.map((cap: Capability) => (
            <li key={cap.name} style={{
              padding: '10px 14px', border: '1px solid #eee', borderRadius: '6px', marginBottom: '8px',
              display: 'flex', justifyContent: 'space-between', alignItems: 'center',
            }}>
              <div>
                <strong>{cap.name}</strong>
                <span style={{ color: '#888', marginLeft: '8px', fontSize: '0.85em' }}>v{cap.version}</span>
                <span style={{ color: '#aaa', marginLeft: '8px', fontSize: '0.8em' }}>:{cap.port}</span>
              </div>
              <span style={{
                background: statusColor(cap.status), color: '#fff',
                borderRadius: '12px', padding: '2px 10px', fontSize: '0.8em',
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
