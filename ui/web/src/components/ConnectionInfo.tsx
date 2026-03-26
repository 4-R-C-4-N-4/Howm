import { useEffect, useCallback } from 'react';
import type { NetworkStatus } from '../api/network';

interface ConnectionInfoProps {
  open: boolean;
  onClose: () => void;
  status: NetworkStatus;
}

export function ConnectionInfo({ open, onClose, status }: ConnectionInfoProps) {
  const handleKeyDown = useCallback((e: KeyboardEvent) => {
    if (e.key === 'Escape') onClose();
  }, [onClose]);

  useEffect(() => {
    if (open) {
      document.addEventListener('keydown', handleKeyDown);
      return () => document.removeEventListener('keydown', handleKeyDown);
    }
  }, [open, handleKeyDown]);

  return (
    <>
      {/* Backdrop */}
      <div
        onClick={onClose}
        className="fixed inset-0 bg-black/40 z-200 transition-opacity duration-200"
        style={{ opacity: open ? 1 : 0, pointerEvents: open ? 'all' : 'none' }}
      />

      {/* Drawer */}
      <div
        className="fixed top-0 right-0 bottom-0 w-[min(400px,100vw)] bg-howm-bg-primary border-l border-howm-border z-[201] transition-transform duration-250 flex flex-col overflow-hidden"
        style={{ transform: open ? 'translateX(0)' : 'translateX(100%)' }}
      >
        <div className="flex items-center gap-3 px-5 py-4 border-b border-howm-border bg-howm-bg-surface shrink-0">
          <button onClick={onClose} className="bg-transparent border-none text-howm-text-secondary text-lg cursor-pointer px-2 py-1 rounded leading-none">✕</button>
          <h2 className="m-0 text-lg font-semibold">Understanding Your Connection</h2>
        </div>

        <div className="flex-1 overflow-y-auto p-5">
          <YourSetup status={status} />
          <HowConnectionsWork status={status} />
          <QuickReference status={status} />
        </div>
      </div>
    </>
  );
}

// ── Section 1: Your Setup ────────────────────────────────────────────────────

function YourSetup({ status }: { status: NetworkStatus }) {
  const { reachability, nat, ipv6, wireguard, peer_count } = status;

  let content: React.ReactNode;

  if (!wireguard.endpoint && !nat?.detected) {
    content = (
      <>
        <p>Your WireGuard endpoint isn't configured yet, and your network type
        hasn't been detected. This means other nodes don't know how to reach you.</p>
        <p>Run <strong>Detect My Network</strong> on the main page — it sends two
        small UDP packets to public STUN servers to figure out your public IP and
        NAT type. Nothing is installed or changed on your system.</p>
      </>
    );
  } else if (reachability === 'direct') {
    const hasIpv6 = ipv6.available && ipv6.global_addresses.length > 0;
    const hasIpv4 = nat?.external_ipv4;
    content = (
      <>
        <p>
          Your node is <strong>directly reachable</strong> from the internet.
          {hasIpv6 && <> You have a public IPv6 address ({ipv6.global_addresses[0]}).</>}
          {hasIpv6 && hasIpv4 && <> You also have IPv4 ({nat!.external_ipv4}).</>}
          {!hasIpv6 && hasIpv4 && <> Your public IP is {nat!.external_ipv4}.</>}
        </p>
        <p>This is the ideal setup. Anyone can connect to you with a simple one-way
        invite — you generate a link, send it to them, they paste it, done.
        No extra steps needed.</p>
      </>
    );
  } else if (reachability === 'punchable') {
    content = (
      <>
        <p>Your node is behind a <strong>cone NAT</strong> — the most common type.
        Connections can be punched through it.</p>
        <p>If you're inviting someone who also has a public IP or IPv6, a simple
        one-way invite works fine. If they're also behind NAT, you'll do a
        two-way exchange — you send them an invite, they send you a response
        link back. Both sides then punch through simultaneously.</p>
        {ipv6.available && (
          <p className="text-sm text-howm-accent mt-2 p-2 bg-[rgba(59,130,246,0.08)] rounded">
            You have IPv6 available — peers with IPv6 can connect to you directly.
          </p>
        )}
      </>
    );
  } else if (reachability === 'relay-only') {
    content = (
      <>
        <p>Your node is behind a <strong>symmetric NAT</strong> — the hardest type
        to connect through. Your router assigns unpredictable ports, which makes
        direct hole-punching unreliable.</p>
        <p>You can still connect directly to anyone who has a public IP or IPv6.
        To connect with someone also behind NAT, you'll need a mutual friend already
        on the mesh who can relay the connection setup. They won't see your traffic —
        they just help you find each other.</p>
        {peer_count === 0 && (
          <p className="text-sm text-howm-warning mt-2 p-2 bg-[rgba(234,179,8,0.08)] rounded">
            You have no peers yet. Your first connection needs to be with someone
            who has a public IP or IPv6 — then they can help bridge you to others.
          </p>
        )}
        {peer_count > 0 && (
          <p className="text-sm text-howm-accent mt-2 p-2 bg-[rgba(59,130,246,0.08)] rounded">
            You have {peer_count} peer{peer_count !== 1 ? 's' : ''} connected.
            {status.relay.relay_capable_peers > 0
              ? ` ${status.relay.relay_capable_peers} of them can relay signaling for you.`
              : ' None of them have relay enabled yet — ask a friend to turn it on in their Connection settings.'}
          </p>
        )}
      </>
    );
  } else {
    // unknown
    content = (
      <>
        <p>Your network type hasn't been detected yet. Howm can figure out the
        best connection strategy for your setup if you run a quick detection.</p>
        <p>It sends two small UDP packets to public STUN servers (Google and Cloudflare) —
        nothing is installed or changed. Takes about 2-3 seconds.</p>
        <p>Without detection, Howm will try its best but can't give you accurate
        guidance about what to expect during connections.</p>
      </>
    );
  }

  return (
    <section className="mb-6">
      <h3 className="text-xs font-bold uppercase tracking-wide text-howm-text-secondary mt-0 mb-3 pb-1.5 border-b border-howm-border">Your Setup</h3>
      <div className="text-sm leading-relaxed text-howm-text-primary">{content}</div>
    </section>
  );
}

// ── Section 2: How Connections Work ──────────────────────────────────────────

function HowConnectionsWork({ status }: { status: NetworkStatus }) {
  const { reachability } = status;

  let inviteFlow: React.ReactNode;
  let redeemFlow: React.ReactNode;

  if (reachability === 'direct' || reachability === 'unknown') {
    inviteFlow = (
      <ol className="mt-1 pl-5 text-sm leading-relaxed">
        <li>Click <strong>Create Invite</strong> to get a link</li>
        <li>Send the link to your friend (text, email, whatever)</li>
        <li>They paste it into their howm → connected!</li>
      </ol>
    );
    redeemFlow = (
      <ol className="mt-1 pl-5 text-sm leading-relaxed">
        <li>They send you a <code className="font-mono text-[0.85em] bg-white/[0.08] px-1 py-px rounded-sm">howm://invite/...</code> link</li>
        <li>Paste it in <strong>Redeem</strong> → connected!</li>
      </ol>
    );
  } else if (reachability === 'punchable') {
    inviteFlow = (
      <>
        <p className="font-semibold text-sm mb-1 text-howm-text-secondary">When the other person has a public IP:</p>
        <ol className="mt-1 pl-5 text-sm leading-relaxed">
          <li>Click <strong>Create Invite</strong> → get a link</li>
          <li>Send it to them → they paste it → connected!</li>
        </ol>
        <p className="font-semibold text-sm mb-1 text-howm-text-secondary">When they're also behind NAT:</p>
        <ol className="mt-1 pl-5 text-sm leading-relaxed">
          <li>Click <strong>Create Invite</strong> → get a link</li>
          <li>Send it to them</li>
          <li>They'll get a response link — they send that back to you</li>
          <li>Paste their response in <strong>Redeem Accept</strong></li>
          <li>Both sides punch through → connected!</li>
        </ol>
        <p className="text-sm text-howm-accent mt-2 p-2 bg-[rgba(59,130,246,0.08)] rounded">
          The two-way exchange is needed so both sides know where to aim. Invites
          expire in 15 minutes, so do the exchange in one sitting.
        </p>
      </>
    );
    redeemFlow = (
      <ol className="mt-1 pl-5 text-sm leading-relaxed">
        <li>They send you a <code className="font-mono text-[0.85em] bg-white/[0.08] px-1 py-px rounded-sm">howm://invite/...</code> link</li>
        <li>Paste it in <strong>Redeem</strong></li>
        <li>If they're behind NAT, you'll get a response link to send back</li>
        <li>Once both sides have the info → connected!</li>
      </ol>
    );
  } else {
    // relay-only
    inviteFlow = (
      <>
        <p className="font-semibold text-sm mb-1 text-howm-text-secondary">To someone with a public IP or IPv6:</p>
        <ol className="mt-1 pl-5 text-sm leading-relaxed">
          <li>Click <strong>Create Invite</strong> → send the link → they paste it → done!</li>
        </ol>
        <p className="font-semibold text-sm mb-1 text-howm-text-secondary">To someone also behind NAT:</p>
        <ol className="mt-1 pl-5 text-sm leading-relaxed">
          <li>You need a mutual friend already on both your meshes</li>
          <li>That friend's node relays the connection setup (not traffic — just
              a few small messages to help you find each other)</li>
          <li>If you have no mutual friends, connect to someone with a public IP first</li>
        </ol>
      </>
    );
    redeemFlow = (
      <ol className="mt-1 pl-5 text-sm leading-relaxed">
        <li>They send you a <code className="font-mono text-[0.85em] bg-white/[0.08] px-1 py-px rounded-sm">howm://invite/...</code> link</li>
        <li>Paste it in <strong>Redeem</strong></li>
        <li>If direct connection fails, howm will try to find a relay path
            through mutual peers</li>
      </ol>
    );
  }

  return (
    <section className="mb-6">
      <h3 className="text-xs font-bold uppercase tracking-wide text-howm-text-secondary mt-0 mb-3 pb-1.5 border-b border-howm-border">How Connections Work For You</h3>
      <div className="text-sm leading-relaxed text-howm-text-primary">
        <p className="font-semibold text-sm mb-1 text-howm-text-secondary">When you invite someone:</p>
        {inviteFlow}
        <div className="mt-4" />
        <p className="font-semibold text-sm mb-1 text-howm-text-secondary">When someone invites you:</p>
        {redeemFlow}
      </div>
    </section>
  );
}

// ── Section 3: Quick Reference ───────────────────────────────────────────────

function QuickReference({ status }: { status: NetworkStatus }) {
  const { reachability } = status;
  const natLabel = reachability === 'direct' ? 'public/open'
    : reachability === 'punchable' ? 'cone'
    : reachability === 'relay-only' ? 'symmetric'
    : 'unknown';

  type Cell = '✓ one-way' | '↔ two-way' | '⚠ relay' | '✕ unreachable' | '? try it';

  let row: [Cell, Cell, Cell];
  if (reachability === 'direct') {
    row = ['✓ one-way', '✓ one-way', '✓ one-way'];
  } else if (reachability === 'punchable') {
    row = ['✓ one-way', '↔ two-way', '↔ two-way'];
  } else if (reachability === 'relay-only') {
    row = ['✓ one-way', '↔ two-way', '⚠ relay'];
  } else {
    row = ['? try it', '? try it', '? try it'];
  }

  const cellColor = (cell: Cell): string => {
    if (cell.startsWith('✓')) return 'var(--howm-success, #22c55e)';
    if (cell.startsWith('↔')) return 'var(--howm-warning, #eab308)';
    if (cell.startsWith('⚠')) return '#fb923c';
    if (cell.startsWith('✕')) return 'var(--howm-error, #ef4444)';
    return 'var(--howm-text-muted, #666666)';
  };

  return (
    <section className="mb-6">
      <h3 className="text-xs font-bold uppercase tracking-wide text-howm-text-secondary mt-0 mb-3 pb-1.5 border-b border-howm-border">Quick Reference</h3>
      <div className="text-sm leading-relaxed text-howm-text-primary">
        <p className="text-howm-text-muted text-sm mb-2.5">
          What happens when you connect to different network types:
        </p>
        <table className="w-full border-collapse text-xs">
          <thead>
            <tr>
              <th className="text-left px-2 py-1.5 border-b border-howm-border text-howm-text-secondary font-semibold text-xs">Them →</th>
              <th className="text-left px-2 py-1.5 border-b border-howm-border text-howm-text-secondary font-semibold text-xs">Public / IPv6</th>
              <th className="text-left px-2 py-1.5 border-b border-howm-border text-howm-text-secondary font-semibold text-xs">Cone NAT</th>
              <th className="text-left px-2 py-1.5 border-b border-howm-border text-howm-text-secondary font-semibold text-xs">Symmetric NAT</th>
            </tr>
          </thead>
          <tbody>
            <tr>
              <td className="px-2 py-1.5 border-b border-howm-border text-xs font-semibold">You ({natLabel})</td>
              <td className="px-2 py-1.5 border-b border-howm-border text-xs" style={{ color: cellColor(row[0]) }}>{row[0]}</td>
              <td className="px-2 py-1.5 border-b border-howm-border text-xs" style={{ color: cellColor(row[1]) }}>{row[1]}</td>
              <td className="px-2 py-1.5 border-b border-howm-border text-xs" style={{ color: cellColor(row[2]) }}>{row[2]}</td>
            </tr>
          </tbody>
        </table>
      </div>
    </section>
  );
}
