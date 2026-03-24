interface DenyModalProps {
  peerName: string;
  onConfirm: () => void;
  onCancel: () => void;
  isPending?: boolean;
}

export function DenyModal({ peerName, onConfirm, onCancel, isPending }: DenyModalProps) {
  return (
    <div style={overlayStyle} onClick={onCancel}>
      <div style={modalStyle} onClick={e => e.stopPropagation()}>
        <div style={{ textAlign: 'center', marginBottom: '16px' }}>
          <div style={{ fontSize: '1.5rem', marginBottom: '8px' }}>⚠️</div>
          <h3 style={{ margin: 0, fontSize: '1.1rem' }}>Deny {peerName}?</h3>
        </div>

        <p style={textStyle}>This will:</p>
        <ul style={{ ...textStyle, paddingLeft: '20px', margin: '8px 0' }}>
          <li>Revoke ALL access immediately</li>
          <li>Close their active P2P-CD session (AuthFailure)</li>
          <li>Remove them from all groups</li>
          <li>They cannot reconnect until you re-add them</li>
        </ul>
        <p style={{ ...textStyle, fontStyle: 'italic', marginTop: '12px', opacity: 0.8 }}>
          {peerName} will notice — their connection will drop.
        </p>

        <div style={{ display: 'flex', gap: '8px', justifyContent: 'center', marginTop: '20px' }}>
          <button onClick={onCancel} style={cancelBtnStyle}>Cancel</button>
          <button onClick={onConfirm} disabled={isPending} style={denyBtnStyle}>
            {isPending ? 'Denying…' : `🔴 Deny ${peerName}`}
          </button>
        </div>
      </div>
    </div>
  );
}

const overlayStyle: React.CSSProperties = {
  position: 'fixed', inset: 0, background: 'rgba(0,0,0,0.6)',
  display: 'flex', alignItems: 'center', justifyContent: 'center',
  zIndex: 250,
};

const modalStyle: React.CSSProperties = {
  background: 'var(--howm-bg-surface, #232733)',
  border: '1px solid var(--howm-border, #2e3341)',
  borderRadius: '12px', padding: '24px',
  maxWidth: '440px', width: '90%',
  boxShadow: '0 16px 48px rgba(0,0,0,0.6)',
};

const textStyle: React.CSSProperties = {
  fontSize: '0.875rem', color: 'var(--howm-text-primary, #e1e4eb)', margin: 0,
};

const cancelBtnStyle: React.CSSProperties = {
  padding: '8px 20px', background: 'var(--howm-bg-elevated, #2a2e3d)',
  border: '1px solid var(--howm-border, #2e3341)',
  borderRadius: '6px', color: 'var(--howm-text-primary, #e1e4eb)',
  cursor: 'pointer', fontSize: '0.9rem',
};

const denyBtnStyle: React.CSSProperties = {
  padding: '8px 20px', background: 'rgba(248,113,113,0.15)',
  border: '1px solid rgba(248,113,113,0.4)',
  borderRadius: '6px', color: '#f87171',
  cursor: 'pointer', fontSize: '0.9rem', fontWeight: 600,
};
