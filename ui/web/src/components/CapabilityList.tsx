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
    if (status === 'Running') return 'var(--howm-success, #22c55e)';
    if (status === 'Stopped') return 'var(--howm-warning, #eab308)';
    return 'var(--howm-error, #ef4444)';
  };

  const statusLabel = (status: Capability['status']) => {
    if (typeof status === 'string') return status;
    if (status && typeof status === 'object' && 'Error' in status) return `Error: ${status.Error}`;
    return 'Unknown';
  };

  if (isLoading) return <p className='text-howm-text-muted m-0 text-sm'>Loading capabilities…</p>;

  return (
    <div>
      <h3 className='m-0 mb-3'>Capabilities ({capabilities.length})</h3>
      {capabilities.length === 0 ? (
        <p className='text-howm-text-muted m-0 text-sm'>No capabilities installed.</p>
      ) : (
        <ul className='list-none p-0 m-0'>
          {capabilities.map((cap: Capability) => (
            <li key={cap.name} className='py-2.5 px-3 border border-howm-border rounded mb-1.5 flex justify-between items-center gap-3 bg-howm-bg-secondary'>
              <div className='flex items-center gap-2 min-w-0'>
                <strong className='whitespace-nowrap'>{cap.name}</strong>
                <span className='text-howm-text-muted m-0 text-sm'>v{cap.version}</span>
                <span className='text-howm-text-muted m-0 text-sm font-mono text-xs'>:{cap.port}</span>

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
