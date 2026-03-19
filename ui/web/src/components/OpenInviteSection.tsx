import { useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { getOpenInvite, createOpenInvite, revokeOpenInvite, redeemOpenInvite } from '../api/nodes';

function extractErrorMessage(err: unknown): string {
  if (err && typeof err === 'object' && 'response' in err) {
    const res = (err as { response?: { data?: { error?: string; message?: string } } }).response;
    if (res?.data?.error) return res.data.error;
    if (res?.data?.message) return res.data.message;
  }
  if (err instanceof Error) return err.message;
  return String(err);
}

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

  if (isLoading) return <p style={mutedStyle}>Loading open invite status…</p>;

  return (
    <div>
      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '12px' }}>
        <h3 style={{ margin: 0 }}>Open Invite</h3>
        <div style={{ display: 'flex', gap: '8px' }}>
          {status?.enabled ? (
            <button onClick={() => revokeMutation.mutate()} style={btnStyle}>
              {revokeMutation.isPending ? 'Revoking…' : 'Revoke'}
            </button>
          ) : (
            <button onClick={() => createMutation.mutate()} style={btnStyle}>
              {createMutation.isPending ? 'Creating…' : 'Create Open Invite'}
            </button>
          )}
          <button onClick={() => setShowRedeem(!showRedeem)} style={btnStyle}>
            Redeem Open Invite
          </button>
        </div>
      </div>

      {/* Task 2: show real error messages */}
      {createMutation.isError && (
        <div style={errorStyle}>
          {extractErrorMessage(createMutation.error)}
        </div>
      )}

      {status?.enabled && status.link && (
        <div style={activeBoxStyle}>
          <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '8px' }}>
            <strong style={{ color: 'var(--howm-success, #4ade80)', fontSize: '0.875rem' }}>● Open Invite Active</strong>
            <span style={mutedStyle}>
              {status.current_peer_count}/{status.max_peers} peers
            </span>
          </div>
          <div style={linkBoxStyle}>
            {status.link}
          </div>
          <button onClick={() => navigator.clipboard?.writeText(status.link!)} style={{ ...btnStyle, marginTop: '10px' }}>
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
          <button onClick={() => redeemMutation.mutate()} disabled={!redeemLink.trim()} style={accentBtnStyle}>
            {redeemMutation.isPending ? 'Redeeming…' : 'Redeem'}
          </button>
          {redeemMutation.isError && (
            <span style={{ color: 'var(--howm-error, #f87171)', fontSize: '0.875em' }}>
              {extractErrorMessage(redeemMutation.error)}
            </span>
          )}
        </div>
      )}

      {!status?.enabled && !showRedeem && (
        <p style={mutedStyle}>
          No open invite active. Create one to share a reusable link with anyone.
        </p>
      )}
    </div>
  );
}

const btnStyle: React.CSSProperties = {
  padding: '6px 14px',
  background: 'var(--howm-bg-elevated, #2a2e3d)',
  border: '1px solid var(--howm-border, #2e3341)',
  borderRadius: 'var(--howm-radius-sm, 4px)',
  color: 'var(--howm-text-primary, #e1e4eb)',
  cursor: 'pointer', fontSize: '0.875em',
};
const accentBtnStyle: React.CSSProperties = {
  padding: '6px 14px',
  background: 'var(--howm-accent, #6c8cff)',
  border: 'none',
  borderRadius: 'var(--howm-radius-sm, 4px)',
  color: '#fff', cursor: 'pointer', fontSize: '0.875em',
};
const formStyle: React.CSSProperties = {
  display: 'flex', gap: '8px', alignItems: 'center', marginBottom: '12px',
  padding: '10px 12px', background: 'var(--howm-bg-secondary, #1a1d27)',
  border: '1px solid var(--howm-border, #2e3341)',
  borderRadius: 'var(--howm-radius-sm, 4px)', flexWrap: 'wrap',
};
const inputStyle: React.CSSProperties = {
  padding: '6px 10px',
  background: 'var(--howm-bg-primary, #0f1117)',
  border: '1px solid var(--howm-border, #2e3341)',
  borderRadius: 'var(--howm-radius-sm, 4px)',
  color: 'var(--howm-text-primary, #e1e4eb)',
  fontSize: '0.875em',
};
const activeBoxStyle: React.CSSProperties = {
  background: 'rgba(74,222,128,0.08)',
  border: '1px solid var(--howm-success, #4ade80)',
  borderRadius: 'var(--howm-radius-sm, 4px)',
  padding: '12px', marginBottom: '12px',
};
const linkBoxStyle: React.CSSProperties = {
  wordBreak: 'break-all',
  fontFamily: 'var(--howm-font-mono, monospace)',
  fontSize: '0.8em',
  background: 'var(--howm-bg-secondary, #1a1d27)',
  padding: '8px 10px',
  borderRadius: 'var(--howm-radius-sm, 4px)',
  border: '1px solid var(--howm-border, #2e3341)',
  color: 'var(--howm-text-primary, #e1e4eb)',
};
const errorStyle: React.CSSProperties = {
  background: 'rgba(248,113,113,0.1)',
  border: '1px solid var(--howm-error, #f87171)',
  borderRadius: 'var(--howm-radius-sm, 4px)',
  padding: '8px 12px', marginBottom: '10px',
  fontSize: '0.875em', color: 'var(--howm-error, #f87171)',
};
const mutedStyle: React.CSSProperties = { color: 'var(--howm-text-muted, #5c6170)', margin: 0, fontSize: '0.875rem' };
