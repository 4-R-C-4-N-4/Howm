interface DenyModalProps {
  peerName: string;
  onConfirm: () => void;
  onCancel: () => void;
  isPending?: boolean;
}

export function DenyModal({ peerName, onConfirm, onCancel, isPending }: DenyModalProps) {
  return (
    <div className="fixed inset-0 bg-black/60 flex items-center justify-center z-[250]" onClick={onCancel}>
      <div className="bg-howm-bg-surface border border-howm-border rounded-xl p-6 max-w-[440px] w-[90%] shadow-[0_16px_48px_rgba(0,0,0,0.6)]" onClick={e => e.stopPropagation()}>
        <div className="text-center mb-4">
          <div className="text-2xl mb-2">⚠️</div>
          <h3 className="m-0 text-lg">Deny {peerName}?</h3>
        </div>

        <p className="text-sm text-howm-text-primary m-0">This will:</p>
        <ul className="text-sm text-howm-text-primary pl-5 my-2">
          <li>Revoke ALL access immediately</li>
          <li>Close their active P2P-CD session (AuthFailure)</li>
          <li>Remove them from all groups</li>
          <li>They cannot reconnect until you re-add them</li>
        </ul>
        <p className="text-sm text-howm-text-primary italic mt-3 opacity-80">
          {peerName} will notice — their connection will drop.
        </p>

        <div className="flex gap-2 justify-center mt-5">
          <button onClick={onCancel} className="py-2 px-5 bg-howm-bg-elevated border border-howm-border rounded-md text-howm-text-primary cursor-pointer text-sm">Cancel</button>
          <button onClick={onConfirm} disabled={isPending}
            className="py-2 px-5 bg-[rgba(239,68,68,0.15)] border border-[rgba(239,68,68,0.4)] rounded-md text-howm-error cursor-pointer text-sm font-semibold">
            {isPending ? 'Denying…' : `🔴 Deny ${peerName}`}
          </button>
        </div>
      </div>
    </div>
  );
}
