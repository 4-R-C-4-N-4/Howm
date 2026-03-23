import { useMutation, useQueryClient } from '@tanstack/react-query';
import { detectNetwork, type NetworkStatus as NetworkStatusType, type Reachability } from '../api/network';

const reachabilityBadge: Record<Reachability, { label: string; color: string; bg: string; icon: string }> = {
  direct:       { label: 'Directly reachable',                    color: 'var(--howm-success, #4ade80)',  bg: 'rgba(74,222,128,0.12)',  icon: '✓' },
  punchable:    { label: 'NAT — two-way exchange for NAT peers',  color: 'var(--howm-warning, #fbbf24)',  bg: 'rgba(251,191,36,0.12)',  icon: '●' },
  'relay-only': { label: 'Symmetric NAT — relay needed for NAT peers', color: '#fb923c', bg: 'rgba(251,146,60,0.12)', icon: '⚠' },
  unknown:      { label: 'Network not yet detected',              color: 'var(--howm-text-muted, #5c6170)', bg: 'rgba(92,97,112,0.12)', icon: '?' },
};

function formatTimestamp(ts: number): string {
  const d = new Date(ts * 1000);
  const now = Date.now();
  const delta = Math.floor((now - ts * 1000) / 1000);
  if (delta < 60) return 'just now';
  if (delta < 3600) return `${Math.floor(delta / 60)}m ago`;
  if (delta < 86400) return `${Math.floor(delta / 3600)}h ago`;
  return d.toLocaleDateString();
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
    <div style={cardStyle}>
      <h2 style={h2Style}>Your Network</h2>

      {/* Endpoint warning */}
      {endpointMissing && (
        <div style={warnBannerStyle}>
          ⚠ WireGuard endpoint not set — invites won't work for remote peers.
          Run network detection below or restart with{' '}
          <code style={codeStyle}>--wg-endpoint &lt;public-ip&gt;:41641</code>.
        </div>
      )}

      {/* Reachability badge */}
      <div style={{ ...badgeContainerStyle, background: badge.bg }}>
        <span style={{ color: badge.color, fontWeight: 600 }}>
          {badge.icon} {badge.label}
        </span>
      </div>

      {/* Status grid */}
      <dl style={dlStyle}>
        <Row label="Status" value={
          <span style={{ color: wg.status === 'connected' ? 'var(--howm-success, #4ade80)' : 'var(--howm-warning, #fbbf24)' }}>
            {wg.status}
          </span>
        } />

        {/* Public IPs */}
        {nat?.external_ipv4 && (
          <Row label="Public IP" value={
            <span style={monoStyle}>{nat.external_ipv4} <span style={tagStyle}>IPv4</span></span>
          } />
        )}
        {ipv6.available && ipv6.global_addresses.map((addr, i) => (
          <Row key={addr} label={i === 0 && !nat?.external_ipv4 ? 'Public IP' : ''} value={
            <span style={monoStyle}>{addr} <span style={{ ...tagStyle, background: 'rgba(74,222,128,0.15)', color: 'var(--howm-success, #4ade80)' }}>IPv6 Global</span></span>
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
                <span style={mutedStyle}> · stride {nat.observed_stride}</span>
              )}
            </span>
          } />
        ) : (
          <Row label="NAT Type" value={<span style={mutedStyle}>Not detected</span>} />
        )}

        {/* WG details */}
        {wg.listen_port != null && <Row label="WG Port" value={<span style={monoStyle}>{wg.listen_port}</span>} />}
        {wg.endpoint && <Row label="Endpoint" value={<span style={monoStyle}>{wg.endpoint}</span>} />}
        {wg.public_key && <Row label="Public Key" value={<span style={{ ...monoStyle, wordBreak: 'break-all' }}>{wg.public_key}</span>} />}
        {wg.active_tunnels != null && <Row label="Active Tunnels" value={<span>{wg.active_tunnels}</span>} />}
      </dl>

      {/* Detection controls */}
      <div style={{ display: 'flex', alignItems: 'center', gap: '12px', marginTop: '16px', paddingTop: '16px', borderTop: '1px solid var(--howm-border, #2e3341)' }}>
        <button onClick={() => detectMutation.mutate()} disabled={detectMutation.isPending} style={btnStyle}>
          {detectMutation.isPending ? 'Detecting…' : nat?.detected ? 'Re-detect Network' : 'Detect My Network'}
        </button>
        {nat?.detected && (
          <span style={{ ...mutedStyle, fontSize: '0.8rem' }}>
            Last detected {formatTimestamp(nat.detected_at)}
          </span>
        )}
        {detectMutation.isError && (
          <span style={{ color: 'var(--howm-error, #f87171)', fontSize: '0.85em' }}>
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
      <dt style={dtStyle}>{label}</dt>
      <dd style={ddStyle}>{value}</dd>
    </>
  );
}

// ── Styles ───────────────────────────────────────────────────────────────────

const cardStyle: React.CSSProperties = {
  background: 'var(--howm-bg-surface, #232733)',
  border: '1px solid var(--howm-border, #2e3341)',
  borderRadius: 'var(--howm-radius-lg, 12px)',
  padding: '20px',
  marginBottom: '20px',
};
const h2Style: React.CSSProperties = {
  fontSize: 'var(--howm-font-size-xl, 1.25rem)',
  fontWeight: 600, marginTop: 0, marginBottom: '16px',
};
const dlStyle: React.CSSProperties = {
  display: 'grid', gridTemplateColumns: 'auto 1fr',
  gap: '6px 16px', margin: 0,
};
const dtStyle: React.CSSProperties = {
  fontWeight: 600, color: 'var(--howm-text-secondary, #8b91a0)',
  fontSize: '0.875rem', alignSelf: 'start', paddingTop: '1px',
  minHeight: dtMinHeight(),
};
const ddStyle: React.CSSProperties = { margin: 0, fontSize: '0.9rem' };
const monoStyle: React.CSSProperties = { fontFamily: 'var(--howm-font-mono, monospace)' };
const mutedStyle: React.CSSProperties = { color: 'var(--howm-text-muted, #5c6170)', fontSize: '0.85em' };
const tagStyle: React.CSSProperties = {
  fontSize: '0.7rem', padding: '1px 6px', borderRadius: '3px',
  background: 'rgba(108,140,255,0.15)', color: 'var(--howm-accent, #6c8cff)',
  marginLeft: '6px', fontFamily: 'var(--howm-font-family, system-ui)',
  verticalAlign: 'middle',
};
const badgeContainerStyle: React.CSSProperties = {
  padding: '8px 12px', borderRadius: 'var(--howm-radius-sm, 4px)',
  marginBottom: '16px', fontSize: '0.875rem',
};
const btnStyle: React.CSSProperties = {
  padding: '6px 14px',
  background: 'var(--howm-bg-elevated, #2a2e3d)',
  border: '1px solid var(--howm-border, #2e3341)',
  borderRadius: 'var(--howm-radius-sm, 4px)',
  color: 'var(--howm-text-primary, #e1e4eb)',
  cursor: 'pointer', fontSize: '0.875em',
};
const warnBannerStyle: React.CSSProperties = {
  background: 'rgba(251,191,36,0.1)',
  border: '1px solid var(--howm-warning, #fbbf24)',
  borderRadius: 'var(--howm-radius-sm, 4px)',
  padding: '10px 14px', marginBottom: '16px',
  fontSize: '0.875rem', color: 'var(--howm-warning, #fbbf24)',
  lineHeight: 1.5,
};
const codeStyle: React.CSSProperties = {
  fontFamily: 'var(--howm-font-mono, monospace)',
  fontSize: '0.875em', background: 'rgba(255,255,255,0.08)',
  padding: '1px 5px', borderRadius: '3px',
};

function dtMinHeight(): string { return '0'; }
