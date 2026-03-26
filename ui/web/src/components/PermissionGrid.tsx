import type { PeerPermissions } from '../api/access';

interface PermissionGridProps {
  permissions: PeerPermissions | undefined;
  isLoading?: boolean;
}

export function PermissionGrid({ permissions, isLoading }: PermissionGridProps) {
  if (isLoading) return <p className="text-howm-text-muted text-sm m-0">Loading permissions…</p>;
  if (!permissions) return <p className="text-howm-text-muted text-sm m-0">No permission data</p>;

  const entries = Object.entries(permissions.permissions);
  const allowed = entries.filter(([, v]) => v.allowed).sort((a, b) => a[0].localeCompare(b[0]));
  const denied = entries.filter(([, v]) => !v.allowed).sort((a, b) => a[0].localeCompare(b[0]));
  const sorted = [...allowed, ...denied];

  if (sorted.length === 0) return <p className="text-howm-text-muted text-sm m-0">No capabilities configured</p>;

  return (
    <div className="flex flex-col gap-1">
      {sorted.map(([cap, perm]) => (
        <div key={cap} className="flex items-center py-1">
          <span style={{ color: perm.allowed ? '#22c55e' : '#ef4444' }} className="mr-2">
            {perm.allowed ? '✓' : '✕'}
          </span>
          <span className="text-sm font-mono">
            {cap}
          </span>
          {perm.rate_limit && (
            <span className="text-xs text-howm-text-muted ml-2">(rate: {perm.rate_limit}/min)</span>
          )}
          {perm.ttl && (
            <span className="text-xs text-howm-text-muted ml-2">(ttl: {perm.ttl}s)</span>
          )}
        </div>
      ))}
    </div>
  );
}
