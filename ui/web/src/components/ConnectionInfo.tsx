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
        style={{
          ...backdropStyle,
          opacity: open ? 1 : 0,
          pointerEvents: open ? 'all' : 'none',
        }}
      />

      {/* Drawer */}
      <div style={{
        ...drawerStyle,
        transform: open ? 'translateX(0)' : 'translateX(100%)',
      }}>
        <div style={drawerHeaderStyle}>
          <button onClick={onClose} style={closeBtnStyle}>✕</button>
          <h2 style={{ margin: 0, fontSize: '1.1rem', fontWeight: 600 }}>
            Understanding Your Connection
          </h2>
        </div>

        <div style={drawerBodyStyle}>
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
          <p style={hintStyle}>
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
          <p style={warnHintStyle}>
            You have no peers yet. Your first connection needs to be with someone
            who has a public IP or IPv6 — then they can help bridge you to others.
          </p>
        )}
        {peer_count > 0 && (
          <p style={hintStyle}>
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
    <section style={sectionStyle}>
      <h3 style={sectionTitleStyle}>Your Setup</h3>
      <div style={sectionBodyStyle}>{content}</div>
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
      <ol style={olStyle}>
        <li>Click <strong>Create Invite</strong> to get a link</li>
        <li>Send the link to your friend (text, email, whatever)</li>
        <li>They paste it into their howm → connected!</li>
      </ol>
    );
    redeemFlow = (
      <ol style={olStyle}>
        <li>They send you a <code style={inlineCodeStyle}>howm://invite/...</code> link</li>
        <li>Paste it in <strong>Redeem</strong> → connected!</li>
      </ol>
    );
  } else if (reachability === 'punchable') {
    inviteFlow = (
      <>
        <p style={subheadStyle}>When the other person has a public IP:</p>
        <ol style={olStyle}>
          <li>Click <strong>Create Invite</strong> → get a link</li>
          <li>Send it to them → they paste it → connected!</li>
        </ol>
        <p style={subheadStyle}>When they're also behind NAT:</p>
        <ol style={olStyle}>
          <li>Click <strong>Create Invite</strong> → get a link</li>
          <li>Send it to them</li>
          <li>They'll get a response link — they send that back to you</li>
          <li>Paste their response in <strong>Redeem Accept</strong></li>
          <li>Both sides punch through → connected!</li>
        </ol>
        <p style={hintStyle}>
          The two-way exchange is needed so both sides know where to aim. Invites
          expire in 15 minutes, so do the exchange in one sitting.
        </p>
      </>
    );
    redeemFlow = (
      <ol style={olStyle}>
        <li>They send you a <code style={inlineCodeStyle}>howm://invite/...</code> link</li>
        <li>Paste it in <strong>Redeem</strong></li>
        <li>If they're behind NAT, you'll get a response link to send back</li>
        <li>Once both sides have the info → connected!</li>
      </ol>
    );
  } else {
    // relay-only
    inviteFlow = (
      <>
        <p style={subheadStyle}>To someone with a public IP or IPv6:</p>
        <ol style={olStyle}>
          <li>Click <strong>Create Invite</strong> → send the link → they paste it → done!</li>
        </ol>
        <p style={subheadStyle}>To someone also behind NAT:</p>
        <ol style={olStyle}>
          <li>You need a mutual friend already on both your meshes</li>
          <li>That friend's node relays the connection setup (not traffic — just
              a few small messages to help you find each other)</li>
          <li>If you have no mutual friends, connect to someone with a public IP first</li>
        </ol>
      </>
    );
    redeemFlow = (
      <ol style={olStyle}>
        <li>They send you a <code style={inlineCodeStyle}>howm://invite/...</code> link</li>
        <li>Paste it in <strong>Redeem</strong></li>
        <li>If direct connection fails, howm will try to find a relay path
            through mutual peers</li>
      </ol>
    );
  }

  return (
    <section style={sectionStyle}>
      <h3 style={sectionTitleStyle}>How Connections Work For You</h3>
      <div style={sectionBodyStyle}>
        <p style={{ ...subheadStyle, marginTop: 0 }}>When you invite someone:</p>
        {inviteFlow}
        <div style={{ marginTop: '16px' }} />
        <p style={subheadStyle}>When someone invites you:</p>
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
    if (cell.startsWith('✓')) return 'var(--howm-success, #4ade80)';
    if (cell.startsWith('↔')) return 'var(--howm-warning, #fbbf24)';
    if (cell.startsWith('⚠')) return '#fb923c';
    if (cell.startsWith('✕')) return 'var(--howm-error, #f87171)';
    return 'var(--howm-text-muted, #5c6170)';
  };

  return (
    <section style={sectionStyle}>
      <h3 style={sectionTitleStyle}>Quick Reference</h3>
      <div style={sectionBodyStyle}>
        <p style={{ ...mutedStyle, marginTop: 0, marginBottom: '10px' }}>
          What happens when you connect to different network types:
        </p>
        <table style={tableStyle}>
          <thead>
            <tr>
              <th style={thStyle}>Them →</th>
              <th style={thStyle}>Public / IPv6</th>
              <th style={thStyle}>Cone NAT</th>
              <th style={thStyle}>Symmetric NAT</th>
            </tr>
          </thead>
          <tbody>
            <tr>
              <td style={{ ...tdStyle, fontWeight: 600 }}>You ({natLabel})</td>
              <td style={{ ...tdStyle, color: cellColor(row[0]) }}>{row[0]}</td>
              <td style={{ ...tdStyle, color: cellColor(row[1]) }}>{row[1]}</td>
              <td style={{ ...tdStyle, color: cellColor(row[2]) }}>{row[2]}</td>
            </tr>
          </tbody>
        </table>
      </div>
    </section>
  );
}

// ── Styles ───────────────────────────────────────────────────────────────────

const backdropStyle: React.CSSProperties = {
  position: 'fixed', inset: 0,
  background: 'rgba(0,0,0,0.4)',
  zIndex: 200,
  transition: 'opacity 0.2s ease',
};

const drawerStyle: React.CSSProperties = {
  position: 'fixed', top: 0, right: 0, bottom: 0,
  width: 'min(400px, 100vw)',
  background: 'var(--howm-bg-primary, #0f1117)',
  borderLeft: '1px solid var(--howm-border, #2e3341)',
  zIndex: 201,
  transition: 'transform 0.25s ease',
  display: 'flex', flexDirection: 'column',
  overflow: 'hidden',
};

const drawerHeaderStyle: React.CSSProperties = {
  display: 'flex', alignItems: 'center', gap: '12px',
  padding: '16px 20px',
  borderBottom: '1px solid var(--howm-border, #2e3341)',
  background: 'var(--howm-bg-surface, #232733)',
  flexShrink: 0,
};

const closeBtnStyle: React.CSSProperties = {
  background: 'none', border: 'none',
  color: 'var(--howm-text-secondary, #8b91a0)',
  fontSize: '1.1rem', cursor: 'pointer',
  padding: '4px 8px', borderRadius: '4px',
  lineHeight: 1,
};

const drawerBodyStyle: React.CSSProperties = {
  flex: 1, overflowY: 'auto',
  padding: '20px',
};

const sectionStyle: React.CSSProperties = {
  marginBottom: '24px',
};

const sectionTitleStyle: React.CSSProperties = {
  fontSize: '0.8rem', fontWeight: 700,
  textTransform: 'uppercase' as const,
  letterSpacing: '0.06em',
  color: 'var(--howm-text-secondary, #8b91a0)',
  marginTop: 0, marginBottom: '12px',
  paddingBottom: '6px',
  borderBottom: '1px solid var(--howm-border, #2e3341)',
};

const sectionBodyStyle: React.CSSProperties = {
  fontSize: '0.9rem',
  lineHeight: 1.6,
  color: 'var(--howm-text-primary, #e1e4eb)',
};

const subheadStyle: React.CSSProperties = {
  fontWeight: 600, fontSize: '0.875rem',
  marginBottom: '4px',
  color: 'var(--howm-text-secondary, #8b91a0)',
};

const olStyle: React.CSSProperties = {
  margin: '4px 0 0', paddingLeft: '20px',
  fontSize: '0.875rem', lineHeight: 1.7,
};

const hintStyle: React.CSSProperties = {
  fontSize: '0.825rem',
  color: 'var(--howm-accent, #6c8cff)',
  marginTop: '8px',
  padding: '8px 10px',
  background: 'rgba(108,140,255,0.08)',
  borderRadius: 'var(--howm-radius-sm, 4px)',
};

const warnHintStyle: React.CSSProperties = {
  ...hintStyle,
  color: 'var(--howm-warning, #fbbf24)',
  background: 'rgba(251,191,36,0.08)',
};

const mutedStyle: React.CSSProperties = {
  color: 'var(--howm-text-muted, #5c6170)',
  fontSize: '0.85rem',
};

const inlineCodeStyle: React.CSSProperties = {
  fontFamily: 'var(--howm-font-mono, monospace)',
  fontSize: '0.85em', background: 'rgba(255,255,255,0.08)',
  padding: '1px 5px', borderRadius: '3px',
};

const tableStyle: React.CSSProperties = {
  width: '100%', borderCollapse: 'collapse',
  fontSize: '0.8rem',
};

const thStyle: React.CSSProperties = {
  textAlign: 'left', padding: '6px 8px',
  borderBottom: '1px solid var(--howm-border, #2e3341)',
  color: 'var(--howm-text-secondary, #8b91a0)',
  fontWeight: 600, fontSize: '0.75rem',
};

const tdStyle: React.CSSProperties = {
  padding: '6px 8px',
  borderBottom: '1px solid var(--howm-border, #2e3341)',
  fontSize: '0.8rem',
};
