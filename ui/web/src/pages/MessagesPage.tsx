import { useQuery } from '@tanstack/react-query';
import { Link } from 'react-router-dom';
import { getConversations, type ConversationSummary } from '../api/messaging';
import { getPeers } from '../api/nodes';

function formatTime(epochMs: number): string {
  const d = new Date(epochMs);
  const now = new Date();
  const sameDay =
    d.getFullYear() === now.getFullYear() &&
    d.getMonth() === now.getMonth() &&
    d.getDate() === now.getDate();
  if (sameDay) return d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
  return d.toLocaleDateString([], { month: 'short', day: 'numeric' });
}

export function MessagesPage() {
  const { data: conversations = [], isLoading } = useQuery({
    queryKey: ['conversations'],
    queryFn: getConversations,
    refetchInterval: 5_000,
  });

  const { data: peers = [] } = useQuery({
    queryKey: ['peers'],
    queryFn: getPeers,
    refetchInterval: 30_000,
  });

  // Build peer name lookup by base64 pubkey
  const peerNames: Record<string, string> = {};
  for (const p of peers) {
    // wg_pubkey is base64 already
    peerNames[p.wg_pubkey] = p.name;
  }

  // Sort by most recent activity
  const sorted = [...conversations].sort((a, b) => {
    const aTime = a.last_message?.sent_at ?? 0;
    const bTime = b.last_message?.sent_at ?? 0;
    return bTime - aTime;
  });

  return (
    <div style={pageStyle}>
      <h2 style={headingStyle}>Messages</h2>

      {isLoading && <p style={mutedStyle}>Loading…</p>}

      {!isLoading && sorted.length === 0 && (
        <p style={mutedStyle}>
          No conversations yet. Send a message from a peer's detail page.
        </p>
      )}

      {sorted.map(conv => (
        <ConversationRow
          key={conv.conversation_id}
          conv={conv}
          peerName={peerNames[conv.peer_id] ?? conv.peer_id.slice(0, 12) + '…'}
        />
      ))}
    </div>
  );
}

function ConversationRow({ conv, peerName }: { conv: ConversationSummary; peerName: string }) {
  const preview = conv.last_message?.body_preview ?? '';
  const time = conv.last_message ? formatTime(conv.last_message.sent_at) : '';
  const prefix = conv.last_message?.direction === 'sent' ? 'You: ' : '';

  return (
    <Link
      to={`/messages/${encodeURIComponent(conv.peer_id)}`}
      style={rowStyle}
    >
      <div style={{ flex: 1, minWidth: 0 }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
          <span style={nameStyle}>{peerName}</span>
          {conv.unread_count > 0 && (
            <span style={badgeStyle}>{conv.unread_count}</span>
          )}
          <span style={{ ...mutedStyle, marginLeft: 'auto', flexShrink: 0, fontSize: '0.8rem' }}>
            {time}
          </span>
        </div>
        <div style={previewStyle}>
          {prefix}{preview || '…'}
        </div>
      </div>
    </Link>
  );
}

// ── Styles ────────────────────────────────────────────────────────────────────

const pageStyle: React.CSSProperties = {
  maxWidth: '640px',
  margin: '0 auto',
  padding: '24px 16px',
};

const headingStyle: React.CSSProperties = {
  color: 'var(--howm-text-primary, #e2e4e9)',
  fontSize: '1.25rem',
  fontWeight: 600,
  marginBottom: '16px',
};

const mutedStyle: React.CSSProperties = {
  color: 'var(--howm-text-muted, #6b7280)',
  fontSize: '0.9rem',
};

const rowStyle: React.CSSProperties = {
  display: 'flex',
  alignItems: 'center',
  padding: '12px 16px',
  background: 'var(--howm-bg-surface, #232733)',
  borderRadius: '8px',
  marginBottom: '6px',
  textDecoration: 'none',
  color: 'inherit',
  cursor: 'pointer',
  transition: 'background 0.15s',
};

const nameStyle: React.CSSProperties = {
  color: 'var(--howm-text-primary, #e2e4e9)',
  fontWeight: 600,
  fontSize: '0.95rem',
};

const previewStyle: React.CSSProperties = {
  color: 'var(--howm-text-secondary, #8b91a0)',
  fontSize: '0.85rem',
  marginTop: '2px',
  overflow: 'hidden',
  textOverflow: 'ellipsis',
  whiteSpace: 'nowrap',
};

const badgeStyle: React.CSSProperties = {
  background: 'var(--howm-accent, #6c8cff)',
  color: '#fff',
  borderRadius: '10px',
  padding: '1px 7px',
  fontSize: '0.75rem',
  fontWeight: 600,
  minWidth: '18px',
  textAlign: 'center',
};
