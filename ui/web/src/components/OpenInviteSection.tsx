import { useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { getOpenInvite, createOpenInvite, revokeOpenInvite, redeemOpenInvite } from '../api/nodes';

export function OpenInviteSection() {
  const queryClient = useQueryClient();
  const { data: status, isLoading } = useQuery({
    queryKey: ['open-invite'],
    queryFn: getOpenInvite,
    refetchInterval: 30000,
  });

  const [redeemLink, setRedeemLink] = useState('');
  const [showRedeem, setShowRedeem] = useState(false);

  const createMutation = useMutation({
    mutationFn: () => createOpenInvite('public'),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['open-invite'] }),
  });

  const revokeMutation = useMutation({
    mutationFn: revokeOpenInvite,
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['open-invite'] }),
  });

  const redeemMutation = useMutation({
    mutationFn: () => redeemOpenInvite(redeemLink),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['peers'] });
      setRedeemLink('');
      setShowRedeem(false);
    },
  });

  if (isLoading) return <p>Loading open invite status...</p>;

  return (
    <div>
      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '12px' }}>
        <h3 style={{ margin: 0 }}>Open Invite</h3>
        <div style={{ display: 'flex', gap: '8px' }}>
          {status?.enabled ? (
            <button onClick={() => revokeMutation.mutate()} style={btnStyle}>
              {revokeMutation.isPending ? 'Revoking...' : 'Revoke'}
            </button>
          ) : (
            <button onClick={() => createMutation.mutate()} style={btnStyle}>
              {createMutation.isPending ? 'Creating...' : 'Create Open Invite'}
            </button>
          )}
          <button onClick={() => setShowRedeem(!showRedeem)} style={btnStyle}>
            Redeem Open Invite
          </button>
        </div>
      </div>

      {createMutation.isError && (
        <div style={{ color: '#ef4444', marginBottom: '8px', fontSize: '0.9em' }}>
          Failed to create open invite. Is the API token set?
        </div>
      )}

      {status?.enabled && status.link && (
        <div style={{ background: '#f0fdf4', border: '1px solid #22c55e', borderRadius: '6px', padding: '12px', marginBottom: '12px' }}>
          <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '8px' }}>
            <strong>Open Invite Active</strong>
            <span style={{ fontSize: '0.85em', color: '#888' }}>
              {status.current_peer_count}/{status.max_peers} peers
            </span>
          </div>
          <div style={{ wordBreak: 'break-all', fontFamily: 'monospace', fontSize: '0.85em', background: '#fff', padding: '8px', borderRadius: '4px', border: '1px solid #e5e7eb' }}>
            {status.link}
          </div>
          <button onClick={() => navigator.clipboard?.writeText(status.link!)} style={{ ...btnStyle, marginTop: '8px' }}>
            Copy Link
          </button>
        </div>
      )}

      {showRedeem && (
        <div style={formStyle}>
          <input
            placeholder="howm://open/..."
            value={redeemLink}
            onChange={e => setRedeemLink(e.target.value)}
            style={{ ...inputStyle, flex: 1 }}
          />
          <button onClick={() => redeemMutation.mutate()} disabled={!redeemLink.trim()} style={btnStyle}>
            {redeemMutation.isPending ? 'Redeeming...' : 'Redeem'}
          </button>
          {redeemMutation.isError && <span style={{ color: 'red', fontSize: '0.9em' }}> Failed — check link and token</span>}
        </div>
      )}

      {!status?.enabled && !showRedeem && (
        <p style={{ color: '#888', fontSize: '0.9em' }}>
          No open invite active. Create one to share a reusable link.
        </p>
      )}
    </div>
  );
}

const btnStyle: React.CSSProperties = {
  padding: '6px 14px', background: '#f3f4f6', border: '1px solid #ddd',
  borderRadius: '6px', cursor: 'pointer', fontSize: '0.9em',
};
const formStyle: React.CSSProperties = {
  display: 'flex', gap: '8px', alignItems: 'center', marginBottom: '12px',
  padding: '10px', background: '#f9fafb', borderRadius: '6px', flexWrap: 'wrap',
};
const inputStyle: React.CSSProperties = {
  padding: '6px 10px', border: '1px solid #ddd', borderRadius: '6px', fontSize: '0.9em',
};
