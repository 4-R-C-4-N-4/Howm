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
    <div className="bg-howm-bg-surface border border-howm-border rounded-xl p-5 mb-5">
      <h2 className="text-xl font-semibold mt-0 mb-4">Invites</h2>

      {/* Action buttons */}
      <div className="flex gap-2 mb-4 flex-wrap">
        <button
          onClick={() => { setActiveTab(activeTab === 'create' ? null : 'create'); setGeneratedInvite(null); }}
          className={`px-3.5 py-1.5 border rounded text-sm cursor-pointer ${
            activeTab === 'create'
              ? 'bg-howm-accent-dim border-howm-accent text-howm-accent'
              : 'bg-howm-bg-elevated border-howm-border text-howm-text-primary'
          }`}
        >
          Create Invite
        </button>
        <button
          onClick={() => setActiveTab(activeTab === 'open' ? null : 'open')}
          className={`px-3.5 py-1.5 border rounded text-sm cursor-pointer ${
            activeTab === 'open'
              ? 'bg-howm-accent-dim border-howm-accent text-howm-accent'
              : 'bg-howm-bg-elevated border-howm-border text-howm-text-primary'
          }`}
        >
          {openStatus?.enabled ? 'Open Invite' : 'Create Open Invite'}
        </button>
        <button
          onClick={() => setActiveTab(activeTab === 'redeem' ? null : 'redeem')}
          className={`px-3.5 py-1.5 border rounded text-sm cursor-pointer ${
            activeTab === 'redeem'
              ? 'bg-howm-accent-dim border-howm-accent text-howm-accent'
              : 'bg-howm-bg-elevated border-howm-border text-howm-text-primary'
          }`}
        >
          Redeem
        </button>
      </div>

      {/* ── Create Invite ─────────────────────────────────────────────────── */}
      {activeTab === 'create' && (
        <div className="p-3.5 px-4 bg-howm-bg-secondary border border-howm-border rounded mb-3">
          {!generatedInvite ? (
            <>
              {reachability === 'unknown' && (
                <p className="text-sm text-howm-accent mt-2 mb-2.5 p-2 bg-[rgba(59,130,246,0.08)] rounded">
                  Run network detection first for the best connection experience.
                </p>
              )}
              <button onClick={() => inviteMutation.mutate()} disabled={inviteMutation.isPending}
                className="px-3.5 py-1.5 bg-howm-accent border-none rounded text-white cursor-pointer text-sm">
                {inviteMutation.isPending ? 'Generating…' : 'Generate Invite Link'}
              </button>
              {inviteMutation.isError && (
                <div className="bg-[rgba(239,68,68,0.1)] border border-howm-error rounded px-3 py-2 mt-2.5 text-sm text-howm-error">
                  {extractErrorMessage(inviteMutation.error)}
                </div>
              )}
            </>
          ) : (
            <>
              <p className="text-sm text-howm-text-secondary mb-2.5 leading-normal">{inviteGuidance()}</p>
              <div className="break-all font-mono text-xs bg-howm-bg-secondary p-2 rounded border border-howm-border text-howm-text-primary">
                {generatedInvite}
              </div>
              <div className="flex gap-2 mt-2.5">
                <button onClick={() => navigator.clipboard?.writeText(generatedInvite)}
                  className="px-3.5 py-1.5 bg-howm-bg-elevated border border-howm-border rounded text-howm-text-primary cursor-pointer text-sm">
                  Copy Link
                </button>
                <button onClick={() => { setGeneratedInvite(null); setActiveTab(null); }}
                  className="px-3.5 py-1.5 bg-howm-bg-elevated border border-howm-border rounded text-howm-text-primary cursor-pointer text-sm">
                  Dismiss
                </button>
              </div>
            </>
          )}
        </div>
      )}

      {/* ── Open Invite ───────────────────────────────────────────────────── */}
      {activeTab === 'open' && (
        <div className="p-3.5 px-4 bg-howm-bg-secondary border border-howm-border rounded mb-3">
          {openStatus?.enabled && openStatus.link ? (
            <div className="bg-[rgba(34,197,94,0.08)] border border-howm-success rounded p-3">
              <div className="flex justify-between items-center mb-2">
                <strong className="text-howm-success text-sm">● Open Invite Active</strong>
                <span className="text-howm-text-muted text-sm">
                  {openStatus.current_peer_count}/{openStatus.max_peers} peers
                </span>
              </div>
              <div className="break-all font-mono text-xs bg-howm-bg-secondary p-2 rounded border border-howm-border text-howm-text-primary">
                {openStatus.link}
              </div>
              <div className="flex gap-2 mt-2.5">
                <button onClick={() => navigator.clipboard?.writeText(openStatus.link!)}
                  className="px-3.5 py-1.5 bg-howm-bg-elevated border border-howm-border rounded text-howm-text-primary cursor-pointer text-sm">
                  Copy Link
                </button>
                <button onClick={() => revokeOpenMutation.mutate()}
                  className="px-3.5 py-1.5 bg-[rgba(239,68,68,0.15)] border border-howm-error rounded text-howm-error cursor-pointer text-sm">
                  {revokeOpenMutation.isPending ? 'Revoking…' : 'Revoke'}
                </button>
              </div>
            </div>
          ) : (
            <>
              <p className="text-howm-text-muted text-sm mb-3">
                Create a reusable invite link that anyone can use to connect to you.
              </p>
              <button onClick={() => createOpenMutation.mutate()} disabled={createOpenMutation.isPending}
                className="px-3.5 py-1.5 bg-howm-accent border-none rounded text-white cursor-pointer text-sm">
                {createOpenMutation.isPending ? 'Creating…' : 'Create Open Invite'}
              </button>
              {createOpenMutation.isError && (
                <div className="bg-[rgba(239,68,68,0.1)] border border-howm-error rounded px-3 py-2 mt-2.5 text-sm text-howm-error">
                  {extractErrorMessage(createOpenMutation.error)}
                </div>
              )}
            </>
          )}
        </div>
      )}

      {/* ── Redeem ────────────────────────────────────────────────────────── */}
      {activeTab === 'redeem' && (
        <div className="p-3.5 px-4 bg-howm-bg-secondary border border-howm-border rounded mb-3">
          <p className="text-howm-text-muted text-sm mb-2.5">
            Paste any invite link — regular, open, or accept response.
          </p>
          <div className="flex gap-2 items-center flex-wrap">
            <input
              placeholder="howm://invite/...  howm://open/...  howm://accept/..."
              value={redeemInput}
              onChange={e => setRedeemInput(e.target.value)}
              onKeyDown={e => e.key === 'Enter' && redeemInput.trim() && handleRedeem()}
              className="flex-1 px-2.5 py-1.5 bg-howm-bg-primary border border-howm-border rounded text-howm-text-primary text-sm font-mono"
            />
            <button
              onClick={handleRedeem}
              disabled={!redeemInput.trim() || redeemMutation.isPending || acceptMutation.isPending || redeemOpenMutation.isPending}
              className="px-3.5 py-1.5 bg-howm-accent border-none rounded text-white cursor-pointer text-sm"
            >
              {(redeemMutation.isPending || acceptMutation.isPending || redeemOpenMutation.isPending)
                ? 'Redeeming…'
                : isAcceptToken ? 'Redeem Accept' : 'Redeem'}
            </button>
          </div>
          {isAcceptToken && (
            <p className="text-sm text-howm-accent mt-2 mb-2.5 p-2 bg-[rgba(59,130,246,0.08)] rounded">
              Detected an accept response — this will complete a two-way exchange.
            </p>
          )}
          {redeemMutation.isError && <div className="bg-[rgba(239,68,68,0.1)] border border-howm-error rounded px-3 py-2 mt-2.5 text-sm text-howm-error">{extractErrorMessage(redeemMutation.error)}</div>}
          {acceptMutation.isError && <div className="bg-[rgba(239,68,68,0.1)] border border-howm-error rounded px-3 py-2 mt-2.5 text-sm text-howm-error">{extractErrorMessage(acceptMutation.error)}</div>}
          {redeemOpenMutation.isError && <div className="bg-[rgba(239,68,68,0.1)] border border-howm-error rounded px-3 py-2 mt-2.5 text-sm text-howm-error">{extractErrorMessage(redeemOpenMutation.error)}</div>}
        </div>
      )}

      {/* ── Pending Exchanges ─────────────────────────────────────────────── */}
      {hasPending && (
        <div className="mt-4">
          <h3 className="text-sm font-semibold mt-0 mb-2.5 text-howm-text-secondary">Pending Exchanges</h3>
          <ul className="list-none p-0 m-0">
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
        <p className="text-howm-text-muted text-sm">
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
    const tick = () => setNow(Math.floor(Date.now() / 1000));
    tick();
    const interval = setInterval(tick, 1000);
    return () => clearInterval(interval);
  }, [exchange.status]);

  const remaining = Math.max(0, exchange.expires_at - now);

  if (exchange.status === 'waiting') {
    return (
      <li className="flex justify-between items-center px-3 py-2.5 border border-howm-border rounded mb-1.5 bg-howm-bg-secondary">
        <div className="flex-1">
          <span className="text-howm-warning font-semibold text-sm">
            ⏳ Waiting for response
          </span>
          <span className="text-howm-text-muted text-sm ml-3">
            Created {formatRelativeTime(exchange.created_at)}
          </span>
          <p className="text-howm-text-muted text-xs mt-1">
            Paste their accept link in Redeem when they send it back.
          </p>
        </div>
        <span className="font-mono text-sm font-semibold whitespace-nowrap" style={{
          color: remaining < 120 ? 'var(--howm-error, #ef4444)' : 'var(--howm-text-secondary, #a0a0a0)',
        }}>
          {formatTimeRemaining(remaining)}
        </span>
      </li>
    );
  }

  if (exchange.status === 'completed') {
    return (
      <li className="flex justify-between items-center px-3 py-2.5 border border-howm-border rounded mb-1.5 bg-howm-bg-secondary">
        <span className="text-howm-success font-semibold text-sm">
          ✓ Connected!
        </span>
        <span className="text-howm-text-muted text-xs ml-3">
          Peer joined via two-way exchange.
        </span>
      </li>
    );
  }

  // expired
  return (
    <li className="flex justify-between items-center px-3 py-2.5 border border-howm-border rounded mb-1.5 bg-howm-bg-secondary">
      <span className="text-howm-text-muted font-semibold text-sm">
        ✕ Expired
      </span>
      <span className="text-howm-text-muted text-xs ml-3">
        No response received. Create a new invite to try again.
      </span>
    </li>
  );
}
