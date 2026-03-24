import type { PeerPermissions } from '../api/access';

interface PermissionGridProps {
  permissions: PeerPermissions | undefined;
  isLoading?: boolean;
}

export function PermissionGrid({ permissions, isLoading }: PermissionGridProps) {
  if (isLoading) return <p style={mutedStyle}>Loading permissions…</p>;
  if (!permissions) return <p style={mutedStyle}>No permission data</p>;

  const entries = Object.entries(permissions.permissions);
  const allowed = entries.filter(([, v]) => v.allowed).sort((a, b) => a[0].localeCompare(b[0]));
  const denied = entries.filter(([, v]) => !v.allowed).sort((a, b) => a[0].localeCompare(b[0]));
  const sorted = [...allowed, ...denied];

  if (sorted.length === 0) return <p style={mutedStyle}>No capabilities configured</p>;

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: '4px' }}>
      {sorted.map(([cap, perm]) => (
        <div key={cap} style={permRowStyle}>
          <span style={{ color: perm.allowed ? '#4ade80' : '#f87171', marginRight: '8px' }}>
            {perm.allowed ? '✓' : '✕'}
          </span>
          <span style={{ fontSize: '0.875rem', fontFamily: 'var(--howm-font-mono, monospace)' }}>
            {cap}
          </span>
          {perm.rate_limit && (
            <span style={subStyle}>(rate: {perm.rate_limit}/min)</span>
          )}
          {perm.ttl && (
            <span style={subStyle}>(ttl: {perm.ttl}s)</span>
          )}
        </div>
      ))}
    </div>
  );
}

const permRowStyle: React.CSSProperties = {
  display: 'flex', alignItems: 'center',
  padding: '4px 0',
};

const subStyle: React.CSSProperties = {
  fontSize: '0.75rem', color: 'var(--howm-text-muted, #5c6170)', marginLeft: '8px',
};

const mutedStyle: React.CSSProperties = {
  color: 'var(--howm-text-muted, #5c6170)', margin: 0, fontSize: '0.875rem',
};
