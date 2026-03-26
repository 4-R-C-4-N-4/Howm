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
    <div className="bg-[rgba(234,179,8,0.08)] border border-[rgba(234,179,8,0.3)] rounded-lg p-4 mt-3">
      <div className="font-semibold mb-2 text-howm-warning">⚠ Warning</div>
      <p className="m-0 text-sm text-howm-text-primary">
        Moving <strong>{peerName}</strong> to {tierLabel} will remove access to:
      </p>
      <div className="my-2">
        {losing.map(cap => (
          <div key={cap} className="text-howm-error text-sm font-mono py-0.5">
            ✕ {cap}
          </div>
        ))}
      </div>
      <p className="text-xs text-howm-text-primary opacity-80 mt-2">
        This takes effect immediately. {peerName}'s active session will be renegotiated.
      </p>
      <div className="flex gap-2 justify-end mt-3">
        <button onClick={onCancel} className="px-3.5 py-1.5 bg-howm-bg-elevated border border-howm-border rounded text-howm-text-primary cursor-pointer text-sm">Cancel</button>
        <button onClick={onConfirm} disabled={isPending}
          className="px-3.5 py-1.5 bg-[rgba(234,179,8,0.15)] border border-[rgba(234,179,8,0.4)] rounded text-howm-warning cursor-pointer text-sm font-semibold">
          {isPending ? 'Updating…' : 'Confirm Demotion'}
        </button>
      </div>
    </div>
  );
}
