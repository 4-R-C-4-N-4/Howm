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
    <div className="bg-howm-bg-surface border border-howm-border rounded-xl p-5 mb-5">
      <h2 className="text-xl font-semibold mt-0 mb-4">Relay</h2>

      <div className="flex justify-between items-center mb-3">
        <span className="font-semibold text-sm">Allow Relay Signaling</span>
        <button onClick={toggle} disabled={mutation.isPending}
          className="py-1 px-4 rounded text-xs font-bold min-w-[52px] tracking-wide cursor-pointer"
          style={{
            border: relay.allow_relay ? '1px solid var(--howm-success, #22c55e)' : '1px solid var(--howm-border, #222222)',
            background: relay.allow_relay ? 'rgba(34,197,94,0.15)' : 'var(--howm-bg-elevated, #1a1a1a)',
            color: relay.allow_relay ? 'var(--howm-success, #22c55e)' : 'var(--howm-text-muted, #666666)',
          }}>
          {mutation.isPending ? '…' : relay.allow_relay ? 'ON' : 'OFF'}
        </button>
      </div>

      <p className="text-howm-text-secondary text-sm leading-relaxed m-0">
        When enabled, your node can help two of your peers who can't reach each
        other directly exchange connection info. No traffic is forwarded — just
        a few small messages to help them find each other.
      </p>

      {relay.allow_relay && relay.relay_capable_peers > 0 && (
        <p className="text-howm-accent text-sm mt-2.5 mb-0 p-1.5 px-2.5 bg-[rgba(59,130,246,0.08)] rounded">
          {relay.relay_capable_peers} of your peers also have relay enabled.
        </p>
      )}

      {relay.allow_relay && relay.relay_capable_peers === 0 && (
        <p className="text-howm-text-muted text-sm mt-2.5 mb-0 p-1.5 px-2.5 bg-[rgba(59,130,246,0.08)] rounded">
          None of your peers have relay enabled yet.
        </p>
      )}

      {mutation.isError && (
        <p className="text-howm-error text-sm mt-2">
          Failed to update relay setting.
        </p>
      )}
    </div>
  );
}
