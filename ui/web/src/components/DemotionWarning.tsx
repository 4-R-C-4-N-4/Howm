import { TIER_CAPABILITIES } from '../api/access';

interface DemotionWarningProps {
  peerName: string;
  targetGroupId: string;
  currentCapabilities: string[];
  onConfirm: () => void;
  onCancel: () => void;
  isPending?: boolean;
}

export function DemotionWarning({ peerName, targetGroupId, currentCapabilities, onConfirm, onCancel, isPending }: DemotionWarningProps) {
  const targetCaps = new Set(TIER_CAPABILITIES[targetGroupId] || []);
  const losing = currentCapabilities.filter(c => !targetCaps.has(c));

  if (losing.length === 0) return null;

  const tierLabel = targetGroupId.endsWith('0001') ? 'Default'
    : targetGroupId.endsWith('0002') ? 'Friends' : 'Trusted';

  return (
    <div style={containerStyle}>
      <div style={headerStyle}>⚠ Warning</div>
      <p style={textStyle}>
        Moving <strong>{peerName}</strong> to {tierLabel} will remove access to:
      </p>
      <div style={{ margin: '8px 0' }}>
        {losing.map(cap => (
          <div key={cap} style={{ color: '#f87171', fontSize: '0.875rem', fontFamily: 'var(--howm-font-mono, monospace)', padding: '2px 0' }}>
            ✕ {cap}
          </div>
        ))}
      </div>
      <p style={{ ...textStyle, fontSize: '0.8rem', opacity: 0.8, marginTop: '8px' }}>
        This takes effect immediately. {peerName}'s active session will be renegotiated.
      </p>
      <div style={{ display: 'flex', gap: '8px', justifyContent: 'flex-end', marginTop: '12px' }}>
        <button onClick={onCancel} style={cancelBtnStyle}>Cancel</button>
        <button onClick={onConfirm} disabled={isPending} style={confirmBtnStyle}>
          {isPending ? 'Updating…' : 'Confirm Demotion'}
        </button>
      </div>
    </div>
  );
}

const containerStyle: React.CSSProperties = {
  background: 'rgba(251,191,36,0.08)',
  border: '1px solid rgba(251,191,36,0.3)',
  borderRadius: '8px', padding: '16px', marginTop: '12px',
};

const headerStyle: React.CSSProperties = {
  fontWeight: 600, marginBottom: '8px', color: '#fbbf24',
};

const textStyle: React.CSSProperties = {
  margin: 0, fontSize: '0.875rem', color: 'var(--howm-text-primary, #e1e4eb)',
};

const cancelBtnStyle: React.CSSProperties = {
  padding: '6px 14px', background: 'var(--howm-bg-elevated, #2a2e3d)',
  border: '1px solid var(--howm-border, #2e3341)',
  borderRadius: '4px', color: 'var(--howm-text-primary, #e1e4eb)',
  cursor: 'pointer', fontSize: '0.85rem',
};

const confirmBtnStyle: React.CSSProperties = {
  padding: '6px 14px', background: 'rgba(251,191,36,0.15)',
  border: '1px solid rgba(251,191,36,0.4)',
  borderRadius: '4px', color: '#fbbf24',
  cursor: 'pointer', fontSize: '0.85rem', fontWeight: 600,
};
