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
    <div className="bg-howm-bg-surface border border-howm-border rounded-lg px-4 py-3 shadow-[0_8px_24px_rgba(0,0,0,0.5)] max-w-[340px]">
      <div className="mb-1.5">
        <strong>🆕 {peer.name}</strong> just joined via invite link
      </div>
      <div className="text-xs text-howm-text-muted mb-2">
        Currently: Default
      </div>
      <div className="flex gap-2">
        <button
          onClick={() => promoteMutation.mutate()}
          disabled={promoteMutation.isPending}
          className="py-1 px-2.5 bg-[rgba(96,165,250,0.15)] border border-[rgba(96,165,250,0.3)] rounded text-[#60a5fa] cursor-pointer text-xs"
        >
          Promote to Friend
        </button>
        <button onClick={() => navigate(`/peers/${hexId}`)}
          className="py-1 px-2.5 bg-howm-bg-elevated border border-howm-border rounded text-howm-text-primary cursor-pointer text-xs">
          View
        </button>
        <button onClick={onDismiss} className="bg-transparent border-none text-howm-text-muted cursor-pointer text-sm ml-auto p-1">✕</button>
      </div>
    </div>
  );
}
