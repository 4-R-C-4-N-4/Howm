import { useState, useEffect } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import {
  getOpenInvite, createOpenInvite, revokeOpenInvite,
  redeemOpenInvite, generateInvite, redeemInvite,
} from '../api/nodes';
import {
  getPendingExchanges, redeemAccept,
  type Reachability, type PendingExchange,
} from '../api/network';

function extractErrorMessage(err: unknown): string {
  if (err && typeof err === 'object' && 'response' in err) {
    const res = (err as { response?: { data?: { error?: string; message?: string } } }).response;
    if (res?.data?.error) return res.data.error;
    if (res?.data?.message) return res.data.message;
  }
  if (err instanceof Error) return err.message;
  return String(err);
}

function formatTimeRemaining(secs: number): string {
  if (secs <= 0) return 'expired';
  const m = Math.floor(secs / 60);
  const s = secs % 60;
  return `${m}:${String(s).padStart(2, '0')}`;
}

function formatRelativeTime(ts: number): string {
  const delta = Math.floor(Date.now() / 1000 - ts);
  if (delta < 60) return 'just now';
  if (delta < 3600) return `${Math.floor(delta / 60)}m ago`;
  return `${Math.floor(delta / 3600)}h ago`;
}

// ── Main Component ───────────────────────────────────────────────────────────

export function InviteManager({ reachability }: { reachability: Reachability }) {
  const qc = useQueryClient();
  const [activeTab, setActiveTab] = useState<'create' | 'open' | 'redeem' | null>(null);
  const [redeemInput, setRedeemInput] = useState('');
  const [acceptInput, setAcceptInput] = useState('');
  const [generatedInvite, setGeneratedInvite] = useState<string | null>(null);

  // Existing open invite status
  const { data: openStatus } = useQuery({
    queryKey: ['open-invite'],
    queryFn: getOpenInvite,
    refetchInterval: 30000,
  });

  // Pending two-way exchanges
  const { data: pending = [] } = useQuery({
    queryKey: ['pending-exchanges'],
    queryFn: getPendingExchanges,
    refetchInterval: 10000,
  });

  // ── Mutations ──────────────────────────────────────────────────────────────

  const inviteMutation = useMutation({
    mutationFn: () => generateInvite(),
    onSuccess: (data) => {
      setGeneratedInvite(data.invite_code);
      qc.invalidateQueries({ queryKey: ['pending-exchanges'] });
    },
  });

  const redeemMutation = useMutation({
    mutationFn: () => redeemInvite(redeemInput),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['peers'] });
      setRedeemInput('');
      setActiveTab(null);
    },
  });

  const acceptMutation = useMutation({
    mutationFn: () => redeemAccept(acceptInput),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['peers'] });
      qc.invalidateQueries({ queryKey: ['pending-exchanges'] });
      setAcceptInput('');
    },
  });

  const createOpenMutation = useMutation({
    mutationFn: () => createOpenInvite('public'),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['open-invite'] }),
  });

  const revokeOpenMutation = useMutation({
    mutationFn: revokeOpenInvite,
    onSuccess: () => qc.invalidateQueries({ queryKey: ['open-invite'] }),
  });

  const redeemOpenMutation = useMutation({
    mutationFn: () => redeemOpenInvite(redeemInput),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['peers'] });
      setRedeemInput('');
      setActiveTab(null);
    },
  });

  // NAT-aware guidance text for invite creation
  const inviteGuidance = (): string | null => {
    if (reachability === 'direct') return 'Send this to your friend. They paste it, you\'re connected.';
    if (reachability === 'punchable') return 'Send this to your friend. If they\'re also behind NAT, they\'ll send you a response link — paste it in Redeem Accept below.';
    if (reachability === 'relay-only') return 'If your friend has a public IP, this will work normally. Otherwise, you\'ll need a mutual friend on the mesh to help with the connection.';
    if (reachability === 'unknown') return 'Tip: Run network detection for better connection guidance.';
    return null;
  };

  // Detect if redeem input looks like an accept token
  const isAcceptToken = redeemInput.startsWith('howm://accept/');
  const isOpenInvite = redeemInput.startsWith('howm://open/');

  const handleRedeem = () => {
    if (isAcceptToken) {
      setAcceptInput(redeemInput);
      setRedeemInput('');
      acceptMutation.mutate();
    } else if (isOpenInvite) {
      redeemOpenMutation.mutate();
    } else {
      redeemMutation.mutate();
    }
  };

  // Active pending exchanges
  const activePending = pending.filter(p => p.status === 'waiting');
  const recentPending = pending.filter(p => p.status !== 'waiting');
  const hasPending = pending.length > 0;

  return (
    <div style={cardStyle}>
      <h2 style={h2Style}>Invites</h2>

      {/* Action buttons */}
      <div style={{ display: 'flex', gap: '8px', marginBottom: '16px', flexWrap: 'wrap' }}>
        <button
          onClick={() => { setActiveTab(activeTab === 'create' ? null : 'create'); setGeneratedInvite(null); }}
          style={activeTab === 'create' ? activeBtnStyle : btnStyle}
        >
          Create Invite
        </button>
        <button
          onClick={() => setActiveTab(activeTab === 'open' ? null : 'open')}
          style={activeTab === 'open' ? activeBtnStyle : btnStyle}
        >
          {openStatus?.enabled ? 'Open Invite' : 'Create Open Invite'}
        </button>
        <button
          onClick={() => setActiveTab(activeTab === 'redeem' ? null : 'redeem')}
          style={activeTab === 'redeem' ? activeBtnStyle : btnStyle}
        >
          Redeem
        </button>
      </div>

      {/* ── Create Invite ─────────────────────────────────────────────────── */}
      {activeTab === 'create' && (
        <div style={panelStyle}>
          {!generatedInvite ? (
            <>
              {reachability === 'unknown' && (
                <p style={hintStyle}>
                  Run network detection first for the best connection experience.
                </p>
              )}
              <button onClick={() => inviteMutation.mutate()} disabled={inviteMutation.isPending} style={accentBtnStyle}>
                {inviteMutation.isPending ? 'Generating…' : 'Generate Invite Link'}
              </button>
              {inviteMutation.isError && (
                <div style={errorStyle}>{extractErrorMessage(inviteMutation.error)}</div>
              )}
            </>
          ) : (
            <>
              <p style={{ ...guidanceStyle, marginTop: 0 }}>{inviteGuidance()}</p>
              <div style={linkBoxStyle}>{generatedInvite}</div>
              <div style={{ display: 'flex', gap: '8px', marginTop: '10px' }}>
                <button onClick={() => navigator.clipboard?.writeText(generatedInvite)} style={btnStyle}>
                  Copy Link
                </button>
                <button onClick={() => { setGeneratedInvite(null); setActiveTab(null); }} style={btnStyle}>
                  Dismiss
                </button>
              </div>
            </>
          )}
        </div>
      )}

      {/* ── Open Invite ───────────────────────────────────────────────────── */}
      {activeTab === 'open' && (
        <div style={panelStyle}>
          {openStatus?.enabled && openStatus.link ? (
            <div style={activeBoxStyle}>
              <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '8px' }}>
                <strong style={{ color: 'var(--howm-success, #4ade80)', fontSize: '0.875rem' }}>● Open Invite Active</strong>
                <span style={mutedStyle}>
                  {openStatus.current_peer_count}/{openStatus.max_peers} peers
                </span>
              </div>
              <div style={linkBoxStyle}>{openStatus.link}</div>
              <div style={{ display: 'flex', gap: '8px', marginTop: '10px' }}>
                <button onClick={() => navigator.clipboard?.writeText(openStatus.link!)} style={btnStyle}>
                  Copy Link
                </button>
                <button onClick={() => revokeOpenMutation.mutate()} style={dangerBtnStyle}>
                  {revokeOpenMutation.isPending ? 'Revoking…' : 'Revoke'}
                </button>
              </div>
            </div>
          ) : (
            <>
              <p style={{ ...mutedStyle, marginTop: 0, marginBottom: '12px' }}>
                Create a reusable invite link that anyone can use to connect to you.
              </p>
              <button onClick={() => createOpenMutation.mutate()} disabled={createOpenMutation.isPending} style={accentBtnStyle}>
                {createOpenMutation.isPending ? 'Creating…' : 'Create Open Invite'}
              </button>
              {createOpenMutation.isError && (
                <div style={errorStyle}>{extractErrorMessage(createOpenMutation.error)}</div>
              )}
            </>
          )}
        </div>
      )}

      {/* ── Redeem ────────────────────────────────────────────────────────── */}
      {activeTab === 'redeem' && (
        <div style={panelStyle}>
          <p style={{ ...mutedStyle, marginTop: 0, marginBottom: '10px' }}>
            Paste any invite link — regular, open, or accept response.
          </p>
          <div style={formStyle}>
            <input
              placeholder="howm://invite/...  howm://open/...  howm://accept/..."
              value={redeemInput}
              onChange={e => setRedeemInput(e.target.value)}
              onKeyDown={e => e.key === 'Enter' && redeemInput.trim() && handleRedeem()}
              style={{ ...inputStyle, flex: 1 }}
            />
            <button
              onClick={handleRedeem}
              disabled={!redeemInput.trim() || redeemMutation.isPending || acceptMutation.isPending || redeemOpenMutation.isPending}
              style={accentBtnStyle}
            >
              {(redeemMutation.isPending || acceptMutation.isPending || redeemOpenMutation.isPending)
                ? 'Redeeming…'
                : isAcceptToken ? 'Redeem Accept' : 'Redeem'}
            </button>
          </div>
          {isAcceptToken && (
            <p style={hintStyle}>Detected an accept response — this will complete a two-way exchange.</p>
          )}
          {redeemMutation.isError && <div style={errorStyle}>{extractErrorMessage(redeemMutation.error)}</div>}
          {acceptMutation.isError && <div style={errorStyle}>{extractErrorMessage(acceptMutation.error)}</div>}
          {redeemOpenMutation.isError && <div style={errorStyle}>{extractErrorMessage(redeemOpenMutation.error)}</div>}
        </div>
      )}

      {/* ── Pending Exchanges ─────────────────────────────────────────────── */}
      {hasPending && (
        <div style={{ marginTop: '16px' }}>
          <h3 style={h3Style}>Pending Exchanges</h3>
          <ul style={{ listStyle: 'none', padding: 0, margin: 0 }}>
            {activePending.map(p => (
              <PendingRow key={p.id} exchange={p} />
            ))}
            {recentPending.map(p => (
              <PendingRow key={p.id} exchange={p} />
            ))}
          </ul>
        </div>
      )}

      {/* Empty state */}
      {activeTab === null && !hasPending && !openStatus?.enabled && (
        <p style={mutedStyle}>
          Create an invite to share with someone, or redeem one you've received.
        </p>
      )}
    </div>
  );
}

// ── Pending Exchange Row ─────────────────────────────────────────────────────

function PendingRow({ exchange }: { exchange: PendingExchange }) {
  const [now, setNow] = useState(() => Math.floor(Date.now() / 1000));

  useEffect(() => {
    if (exchange.status !== 'waiting') return;
    setNow(Math.floor(Date.now() / 1000));
    const interval = setInterval(() => setNow(Math.floor(Date.now() / 1000)), 1000);
    return () => clearInterval(interval);
  }, [exchange.status]);

  const remaining = Math.max(0, exchange.expires_at - now);

  if (exchange.status === 'waiting') {
    return (
      <li style={pendingRowStyle}>
        <div style={{ flex: 1 }}>
          <span style={{ color: 'var(--howm-warning, #fbbf24)', fontWeight: 600, fontSize: '0.875rem' }}>
            ⏳ Waiting for response
          </span>
          <span style={{ ...mutedStyle, marginLeft: '12px' }}>
            Created {formatRelativeTime(exchange.created_at)}
          </span>
          <p style={{ ...mutedStyle, margin: '4px 0 0', fontSize: '0.825rem' }}>
            Paste their accept link in Redeem when they send it back.
          </p>
        </div>
        <span style={{
          fontFamily: 'var(--howm-font-mono, monospace)',
          fontSize: '0.85rem',
          color: remaining < 120 ? 'var(--howm-error, #f87171)' : 'var(--howm-text-secondary, #8b91a0)',
          fontWeight: 600, whiteSpace: 'nowrap',
        }}>
          {formatTimeRemaining(remaining)}
        </span>
      </li>
    );
  }

  if (exchange.status === 'completed') {
    return (
      <li style={pendingRowStyle}>
        <span style={{ color: 'var(--howm-success, #4ade80)', fontWeight: 600, fontSize: '0.875rem' }}>
          ✓ Connected!
        </span>
        <span style={{ ...mutedStyle, marginLeft: '12px', fontSize: '0.825rem' }}>
          Peer joined via two-way exchange.
        </span>
      </li>
    );
  }

  // expired
  return (
    <li style={pendingRowStyle}>
      <span style={{ color: 'var(--howm-text-muted, #5c6170)', fontWeight: 600, fontSize: '0.875rem' }}>
        ✕ Expired
      </span>
      <span style={{ ...mutedStyle, marginLeft: '12px', fontSize: '0.825rem' }}>
        No response received. Create a new invite to try again.
      </span>
    </li>
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
const h3Style: React.CSSProperties = {
  fontSize: '0.95rem', fontWeight: 600,
  marginTop: 0, marginBottom: '10px',
  color: 'var(--howm-text-secondary, #8b91a0)',
};
const btnStyle: React.CSSProperties = {
  padding: '6px 14px',
  background: 'var(--howm-bg-elevated, #2a2e3d)',
  border: '1px solid var(--howm-border, #2e3341)',
  borderRadius: 'var(--howm-radius-sm, 4px)',
  color: 'var(--howm-text-primary, #e1e4eb)',
  cursor: 'pointer', fontSize: '0.875em',
};
const activeBtnStyle: React.CSSProperties = {
  ...btnStyle,
  background: 'rgba(108,140,255,0.15)',
  borderColor: 'var(--howm-accent, #6c8cff)',
  color: 'var(--howm-accent, #6c8cff)',
};
const accentBtnStyle: React.CSSProperties = {
  padding: '6px 14px',
  background: 'var(--howm-accent, #6c8cff)',
  border: 'none',
  borderRadius: 'var(--howm-radius-sm, 4px)',
  color: '#fff', cursor: 'pointer', fontSize: '0.875em',
};
const dangerBtnStyle: React.CSSProperties = {
  ...btnStyle,
  background: 'rgba(248,113,113,0.15)',
  color: 'var(--howm-error, #f87171)',
  border: '1px solid var(--howm-error, #f87171)',
};
const panelStyle: React.CSSProperties = {
  padding: '14px 16px',
  background: 'var(--howm-bg-secondary, #1a1d27)',
  border: '1px solid var(--howm-border, #2e3341)',
  borderRadius: 'var(--howm-radius-sm, 4px)',
  marginBottom: '12px',
};
const formStyle: React.CSSProperties = {
  display: 'flex', gap: '8px', alignItems: 'center', flexWrap: 'wrap',
};
const inputStyle: React.CSSProperties = {
  padding: '6px 10px',
  background: 'var(--howm-bg-primary, #0f1117)',
  border: '1px solid var(--howm-border, #2e3341)',
  borderRadius: 'var(--howm-radius-sm, 4px)',
  color: 'var(--howm-text-primary, #e1e4eb)',
  fontSize: '0.875em', fontFamily: 'var(--howm-font-mono, monospace)',
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
const activeBoxStyle: React.CSSProperties = {
  background: 'rgba(74,222,128,0.08)',
  border: '1px solid var(--howm-success, #4ade80)',
  borderRadius: 'var(--howm-radius-sm, 4px)',
  padding: '12px',
};
const errorStyle: React.CSSProperties = {
  background: 'rgba(248,113,113,0.1)',
  border: '1px solid var(--howm-error, #f87171)',
  borderRadius: 'var(--howm-radius-sm, 4px)',
  padding: '8px 12px', marginTop: '10px',
  fontSize: '0.875em', color: 'var(--howm-error, #f87171)',
};
const hintStyle: React.CSSProperties = {
  fontSize: '0.825rem',
  color: 'var(--howm-accent, #6c8cff)',
  marginTop: '8px', marginBottom: '10px',
  padding: '8px 10px',
  background: 'rgba(108,140,255,0.08)',
  borderRadius: 'var(--howm-radius-sm, 4px)',
};
const guidanceStyle: React.CSSProperties = {
  fontSize: '0.875rem',
  color: 'var(--howm-text-secondary, #8b91a0)',
  marginBottom: '10px', lineHeight: 1.5,
};
const mutedStyle: React.CSSProperties = {
  color: 'var(--howm-text-muted, #5c6170)',
  margin: 0, fontSize: '0.875rem',
};
const pendingRowStyle: React.CSSProperties = {
  display: 'flex', justifyContent: 'space-between', alignItems: 'center',
  padding: '10px 12px',
  border: '1px solid var(--howm-border, #2e3341)',
  borderRadius: 'var(--howm-radius-sm, 4px)',
  marginBottom: '6px',
  background: 'var(--howm-bg-secondary, #1a1d27)',
};
