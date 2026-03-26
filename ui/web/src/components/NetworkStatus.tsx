import { useMutation, useQueryClient } from '@tanstack/react-query';
import { detectNetwork, type NetworkStatus as NetworkStatusType, type Reachability } from '../api/network';

const reachabilityBadge: Record<Reachability, { label: string; color: string; bg: string; icon: string }> = {
  direct:       { label: 'Directly reachable',                    color: 'var(--howm-success, #22c55e)',  bg: 'rgba(34,197,94,0.12)',  icon: '✓' },
  punchable:    { label: 'NAT — two-way exchange for NAT peers',  color: 'var(--howm-warning, #eab308)',  bg: 'rgba(234,179,8,0.12)',  icon: '●' },
  'relay-only': { label: 'Symmetric NAT — relay needed for NAT peers', color: '#fb923c', bg: 'rgba(251,146,60,0.12)', icon: '⚠' },
  unknown:      { label: 'Network not yet detected',              color: 'var(--howm-text-muted, #666666)', bg: 'rgba(102,102,102,0.12)', icon: '?' },
};

function formatTimestamp(ts: number): string {
  const now = Date.now();
  const delta = Math.floor((now - ts * 1000) / 1000);
  if (delta < 60) return 'just now';
  if (delta < 3600) return `${Math.floor(delta / 60)}m ago`;
  if (delta < 86400) return `${Math.floor(delta / 3600)}h ago`;
  return new Date(ts * 1000).toLocaleDateString();
}

export function NetworkStatus({ status }: { status: NetworkStatusType }) {
  const qc = useQueryClient();
  const badge = reachabilityBadge[status.reachability];
  const wg = status.wireguard;
  const nat = status.nat;
  const ipv6 = status.ipv6;

  const detectMutation = useMutation({
    mutationFn: detectNetwork,
    onSuccess: () => qc.invalidateQueries({ queryKey: ['network-status'] }),
  });

  const endpointMissing = wg && !wg.endpoint;

  return (
    <div className="bg-howm-bg-surface border border-howm-border rounded-xl p-5 mb-5">
      <h2 className="text-xl font-semibold mt-0 mb-4">Your Network</h2>

      {/* Endpoint warning */}
      {endpointMissing && (
        <div className="bg-[rgba(234,179,8,0.1)] border border-howm-warning rounded p-3.5 mb-4 text-sm text-howm-warning leading-normal">
          ⚠ WireGuard endpoint not set — invites won't work for remote peers.
          Run network detection below or restart with{' '}
          <code className="font-mono text-sm bg-white/[0.08] px-1 py-px rounded-sm">--wg-endpoint &lt;public-ip&gt;:41641</code>.
        </div>
      )}

      {/* Reachability badge */}
      <div className="p-2 px-3 rounded mb-4 text-sm" style={{ background: badge.bg }}>
        <span style={{ color: badge.color }} className="font-semibold">
          {badge.icon} {badge.label}
        </span>
      </div>

      {/* Status grid */}
      <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1.5 m-0">
        <Row label="Status" value={
          <span style={{ color: wg.status === 'connected' ? 'var(--howm-success, #22c55e)' : 'var(--howm-warning, #eab308)' }}>
            {wg.status}
          </span>
        } />

        {/* Public IPs */}
        {nat?.external_ipv4 && (
          <Row label="Public IP" value={
            <span className="font-mono">{nat.external_ipv4} <span className="text-[0.7rem] px-1.5 py-px rounded-sm bg-howm-accent-dim text-howm-accent ml-1.5 font-sans align-middle">IPv4</span></span>
          } />
        )}
        {ipv6.available && ipv6.global_addresses.map((addr, i) => (
          <Row key={addr} label={i === 0 && !nat?.external_ipv4 ? 'Public IP' : ''} value={
            <span className="font-mono">{addr} <span className="text-[0.7rem] px-1.5 py-px rounded-sm bg-[rgba(34,197,94,0.15)] text-howm-success ml-1.5 font-sans align-middle">IPv6 Global</span></span>
          } />
        ))}

        {/* NAT type */}
        {nat?.detected ? (
          <Row label="NAT Type" value={
            <span>
              {nat.nat_type === 'open' ? 'Open (no NAT)' :
               nat.nat_type === 'cone' ? 'Cone (port-preserving)' :
               nat.nat_type === 'symmetric' ? 'Symmetric' : 'Unknown'}
              {nat.observed_stride !== 0 && (
                <span className="text-howm-text-muted text-xs"> · stride {nat.observed_stride}</span>
              )}
            </span>
          } />
        ) : (
          <Row label="NAT Type" value={<span className="text-howm-text-muted text-xs">Not detected</span>} />
        )}

        {/* WG details */}
        {wg.listen_port != null && <Row label="WG Port" value={<span className="font-mono">{wg.listen_port}</span>} />}
        {wg.endpoint && <Row label="Endpoint" value={<span className="font-mono">{wg.endpoint}</span>} />}
        {wg.public_key && <Row label="Public Key" value={<span className="font-mono break-all">{wg.public_key}</span>} />}
        {wg.active_tunnels != null && <Row label="Active Tunnels" value={<span>{wg.active_tunnels}</span>} />}
      </dl>

      {/* Detection controls */}
      <div className="flex items-center gap-3 mt-4 pt-4 border-t border-howm-border">
        <button onClick={() => detectMutation.mutate()} disabled={detectMutation.isPending}
          className="px-3.5 py-1.5 bg-howm-bg-elevated border border-howm-border rounded text-howm-text-primary cursor-pointer text-sm">
          {detectMutation.isPending ? 'Detecting…' : nat?.detected ? 'Re-detect Network' : 'Detect My Network'}
        </button>
        {nat?.detected && (
          <span className="text-howm-text-muted text-xs">
            Last detected {formatTimestamp(nat.detected_at)}
          </span>
        )}
        {detectMutation.isError && (
          <span className="text-howm-error text-sm">
            Detection failed
          </span>
        )}
      </div>
    </div>
  );
}

function Row({ label, value }: { label: string; value: React.ReactNode }) {
  return (
    <>
      <dt className="font-semibold text-howm-text-secondary text-sm self-start pt-px">{label}</dt>
      <dd className="m-0 text-sm">{value}</dd>
    </>
  );
}
