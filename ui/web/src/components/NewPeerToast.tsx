import { useNavigate } from 'react-router-dom';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { movePeerToTier, GROUP_FRIENDS } from '../api/access';
import { peerIdToHex } from '../lib/access';

interface NewPeerToastProps {
  peer: { name: string; wg_pubkey: string };
  onDismiss: () => void;
}

export function NewPeerToast({ peer, onDismiss }: NewPeerToastProps) {
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const hexId = peerIdToHex(peer.wg_pubkey);

  const promoteMutation = useMutation({
    mutationFn: () => movePeerToTier(hexId, GROUP_FRIENDS),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['peer-groups'] });
      queryClient.invalidateQueries({ queryKey: ['peers'] });
      onDismiss();
    },
  });

  return (
    <div style={toastStyle}>
      <div style={{ marginBottom: '6px' }}>
        <strong>🆕 {peer.name}</strong> just joined via invite link
      </div>
      <div style={{ fontSize: '0.8rem', color: 'var(--howm-text-muted, #5c6170)', marginBottom: '8px' }}>
        Currently: Default
      </div>
      <div style={{ display: 'flex', gap: '8px' }}>
        <button
          onClick={() => promoteMutation.mutate()}
          disabled={promoteMutation.isPending}
          style={promoteBtnStyle}
        >
          Promote to Friend
        </button>
        <button onClick={() => navigate(`/peers/${hexId}`)} style={viewBtnStyle}>
          View
        </button>
        <button onClick={onDismiss} style={dismissBtnStyle}>✕</button>
      </div>
    </div>
  );
}

const toastStyle: React.CSSProperties = {
  background: 'var(--howm-bg-surface, #232733)',
  border: '1px solid var(--howm-border, #2e3341)',
  borderRadius: '8px', padding: '12px 16px',
  boxShadow: '0 8px 24px rgba(0,0,0,0.5)',
  maxWidth: '340px',
};
const promoteBtnStyle: React.CSSProperties = {
  padding: '4px 10px', background: 'rgba(96,165,250,0.15)',
  border: '1px solid rgba(96,165,250,0.3)', borderRadius: '4px',
  color: '#60a5fa', cursor: 'pointer', fontSize: '0.8rem',
};
const viewBtnStyle: React.CSSProperties = {
  padding: '4px 10px', background: 'var(--howm-bg-elevated, #2a2e3d)',
  border: '1px solid var(--howm-border, #2e3341)', borderRadius: '4px',
  color: 'var(--howm-text-primary, #e1e4eb)', cursor: 'pointer', fontSize: '0.8rem',
};
const dismissBtnStyle: React.CSSProperties = {
  background: 'none', border: 'none', color: 'var(--howm-text-muted, #5c6170)',
  cursor: 'pointer', fontSize: '0.9rem', marginLeft: 'auto', padding: '4px',
};
