import { useMutation, useQueryClient } from '@tanstack/react-query';
import { updateRelayConfig, type RelayConfig as RelayConfigType } from '../api/network';

export function RelayConfig({ relay }: { relay: RelayConfigType }) {
  const qc = useQueryClient();

  const mutation = useMutation({
    mutationFn: (allow: boolean) => updateRelayConfig(allow),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['network-status'] }),
  });

  const toggle = () => mutation.mutate(!relay.allow_relay);

  return (
    <div style={cardStyle}>
      <h2 style={h2Style}>Relay</h2>

      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '12px' }}>
        <span style={{ fontWeight: 600, fontSize: '0.9rem' }}>
          Allow Relay Signaling
        </span>
        <button onClick={toggle} disabled={mutation.isPending} style={toggleBtnStyle(relay.allow_relay)}>
          {mutation.isPending ? '…' : relay.allow_relay ? 'ON' : 'OFF'}
        </button>
      </div>

      <p style={descStyle}>
        When enabled, your node can help two of your peers who can't reach each
        other directly exchange connection info. No traffic is forwarded — just
        a few small messages to help them find each other.
      </p>

      {relay.allow_relay && relay.relay_capable_peers > 0 && (
        <p style={statStyle}>
          {relay.relay_capable_peers} of your peers also have relay enabled.
        </p>
      )}

      {relay.allow_relay && relay.relay_capable_peers === 0 && (
        <p style={{ ...statStyle, color: 'var(--howm-text-muted, #5c6170)' }}>
          None of your peers have relay enabled yet.
        </p>
      )}

      {mutation.isError && (
        <p style={{ color: 'var(--howm-error, #f87171)', fontSize: '0.85em', marginTop: '8px' }}>
          Failed to update relay setting.
        </p>
      )}
    </div>
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
const descStyle: React.CSSProperties = {
  color: 'var(--howm-text-secondary, #8b91a0)',
  fontSize: '0.875rem', lineHeight: 1.6,
  margin: 0,
};
const statStyle: React.CSSProperties = {
  color: 'var(--howm-accent, #6c8cff)',
  fontSize: '0.825rem',
  marginTop: '10px', marginBottom: 0,
  padding: '6px 10px',
  background: 'rgba(108,140,255,0.08)',
  borderRadius: 'var(--howm-radius-sm, 4px)',
};

function toggleBtnStyle(on: boolean): React.CSSProperties {
  return {
    padding: '4px 16px',
    borderRadius: 'var(--howm-radius-sm, 4px)',
    border: on
      ? '1px solid var(--howm-success, #4ade80)'
      : '1px solid var(--howm-border, #2e3341)',
    background: on
      ? 'rgba(74,222,128,0.15)'
      : 'var(--howm-bg-elevated, #2a2e3d)',
    color: on
      ? 'var(--howm-success, #4ade80)'
      : 'var(--howm-text-muted, #5c6170)',
    fontWeight: 700,
    fontSize: '0.8rem',
    cursor: 'pointer',
    minWidth: '52px',
    letterSpacing: '0.04em',
  };
}
