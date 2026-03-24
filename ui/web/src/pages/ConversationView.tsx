import { useParams, Link } from 'react-router-dom';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { useState, useEffect, useRef, useCallback } from 'react';
import {
  getConversation,
  sendMessage,
  markRead,
  type Message,
} from '../api/messaging';
import { getPeers } from '../api/nodes';

function formatTimestamp(epochMs: number): string {
  const d = new Date(epochMs);
  return d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
}

function formatDate(epochMs: number): string {
  const d = new Date(epochMs);
  const now = new Date();
  const sameDay =
    d.getFullYear() === now.getFullYear() &&
    d.getMonth() === now.getMonth() &&
    d.getDate() === now.getDate();
  if (sameDay) return 'Today';
  const yesterday = new Date(now);
  yesterday.setDate(yesterday.getDate() - 1);
  if (
    d.getFullYear() === yesterday.getFullYear() &&
    d.getMonth() === yesterday.getMonth() &&
    d.getDate() === yesterday.getDate()
  )
    return 'Yesterday';
  return d.toLocaleDateString([], { month: 'short', day: 'numeric', year: 'numeric' });
}

export function ConversationView() {
  const { peerId } = useParams<{ peerId: string }>();
  const queryClient = useQueryClient();
  const [body, setBody] = useState('');
  const [optimistic, setOptimistic] = useState<Message[]>([]);
  const scrollRef = useRef<HTMLDivElement>(null);
  const composerRef = useRef<HTMLTextAreaElement>(null);

  const decodedPeerId = peerId ? decodeURIComponent(peerId) : '';

  // Fetch messages (poll every 3s)
  const { data, isLoading } = useQuery({
    queryKey: ['conversation', decodedPeerId],
    queryFn: () => getConversation(decodedPeerId),
    refetchInterval: 3_000,
    enabled: !!decodedPeerId,
  });

  // Peer name lookup
  const { data: peers = [] } = useQuery({
    queryKey: ['peers'],
    queryFn: getPeers,
    refetchInterval: 30_000,
  });

  const peerName = peers.find(p => p.wg_pubkey === decodedPeerId)?.name
    ?? decodedPeerId.slice(0, 12) + '…';
  const peerOnline = peers.some(p => p.wg_pubkey === decodedPeerId && Date.now() - p.last_seen * 1000 < 120_000);

  // Mark as read on open
  useEffect(() => {
    if (decodedPeerId) {
      markRead(decodedPeerId).catch(() => {});
    }
  }, [decodedPeerId]);

  // Also mark read when new messages arrive
  useEffect(() => {
    if (data?.messages?.some(m => m.direction === 'received')) {
      markRead(decodedPeerId).catch(() => {});
      queryClient.invalidateQueries({ queryKey: ['conversations'] });
    }
  }, [data, decodedPeerId, queryClient]);

  // Merge server messages with optimistic ones
  const serverMsgs = data?.messages ?? [];
  const serverIds = new Set(serverMsgs.map(m => m.msg_id));
  const allMessages = [
    ...serverMsgs,
    ...optimistic.filter(m => !serverIds.has(m.msg_id)),
  ].sort((a, b) => a.sent_at - b.sent_at);

  // Clean up delivered optimistic messages
  useEffect(() => {
    if (optimistic.length > 0) {
      setOptimistic(prev => prev.filter(m => !serverIds.has(m.msg_id)));
    }
  }, [serverIds.size]); // eslint-disable-line react-hooks/exhaustive-deps

  // Auto-scroll to bottom
  useEffect(() => {
    if (scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [allMessages.length]);

  // Send mutation
  const sendMut = useMutation({
    mutationFn: ({ to, body: b }: { to: string; body: string }) => sendMessage(to, b),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['conversation', decodedPeerId] });
      queryClient.invalidateQueries({ queryKey: ['conversations'] });
    },
  });

  const handleSend = useCallback(() => {
    const trimmed = body.trim();
    if (!trimmed || trimmed.length > 4096) return;

    // Optimistic insert
    const tempId = 'opt-' + Date.now();
    const optMsg: Message = {
      msg_id: tempId,
      conversation_id: '',
      direction: 'sent',
      sender_peer_id: '',
      sent_at: Date.now(),
      body: trimmed,
      delivery_status: 'pending',
    };
    setOptimistic(prev => [...prev, optMsg]);
    setBody('');

    sendMut.mutate({ to: decodedPeerId, body: trimmed });
  }, [body, decodedPeerId, sendMut]);

  const handleKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      handleSend();
    }
  };

  const byteLen = new TextEncoder().encode(body).length;
  const overLimit = byteLen > 4096;
  const nearLimit = byteLen > 4000;

  // Group messages by date
  let lastDate = '';

  return (
    <div style={pageStyle}>
      {/* Header */}
      <div style={headerStyle}>
        <Link to="/messages" style={backStyle}>←</Link>
        <span style={headerNameStyle}>{peerName}</span>
        {peerOnline && <span style={onlineDotStyle} />}
      </div>

      {/* Messages */}
      <div ref={scrollRef} style={messagesContainerStyle}>
        {isLoading && <p style={mutedStyle}>Loading…</p>}

        {allMessages.map(msg => {
          const dateStr = formatDate(msg.sent_at);
          const showDate = dateStr !== lastDate;
          lastDate = dateStr;
          const isSent = msg.direction === 'sent';

          return (
            <div key={msg.msg_id}>
              {showDate && <div style={dateDividerStyle}>{dateStr}</div>}
              <div style={{ display: 'flex', justifyContent: isSent ? 'flex-end' : 'flex-start', marginBottom: '4px' }}>
                <div style={isSent ? sentBubbleStyle : receivedBubbleStyle}>
                  <div style={{ whiteSpace: 'pre-wrap', wordBreak: 'break-word' }}>{msg.body}</div>
                  <div style={metaStyle}>
                    {formatTimestamp(msg.sent_at)}
                    {isSent && (
                      <span style={{ marginLeft: '6px' }}>
                        {msg.delivery_status === 'pending' && '⏳'}
                        {msg.delivery_status === 'delivered' && '✓'}
                        {msg.delivery_status === 'failed' && (
                          <span title="Delivery failed" style={{ cursor: 'help' }}>⚠</span>
                        )}
                      </span>
                    )}
                  </div>
                </div>
              </div>
            </div>
          );
        })}
      </div>

      {/* Composer */}
      <div style={composerContainerStyle}>
        {!peerOnline && (
          <div style={offlineBannerStyle}>
            Peer is offline — messages cannot be delivered right now
          </div>
        )}
        <div style={{ display: 'flex', gap: '8px', alignItems: 'flex-end' }}>
          <textarea
            ref={composerRef}
            value={body}
            onChange={e => setBody(e.target.value)}
            onKeyDown={handleKeyDown}
            placeholder={peerOnline ? 'Type a message…' : 'Peer is offline'}
            disabled={!peerOnline}
            rows={1}
            style={{
              ...textareaStyle,
              opacity: peerOnline ? 1 : 0.5,
            }}
          />
          <button
            onClick={handleSend}
            disabled={!body.trim() || overLimit || !peerOnline || sendMut.isPending}
            style={{
              ...sendBtnStyle,
              opacity: (!body.trim() || overLimit || !peerOnline) ? 0.4 : 1,
            }}
          >
            Send
          </button>
        </div>
        <div style={{
          ...counterStyle,
          color: overLimit ? '#ef4444' : nearLimit ? '#f59e0b' : 'var(--howm-text-muted, #6b7280)',
        }}>
          {byteLen} / 4096
        </div>
      </div>
    </div>
  );
}

// ── Styles ────────────────────────────────────────────────────────────────────

const pageStyle: React.CSSProperties = {
  display: 'flex',
  flexDirection: 'column',
  height: 'calc(100vh - 48px)',
  maxWidth: '720px',
  margin: '0 auto',
};

const headerStyle: React.CSSProperties = {
  display: 'flex',
  alignItems: 'center',
  gap: '12px',
  padding: '12px 16px',
  borderBottom: '1px solid var(--howm-border, #2e3341)',
  background: 'var(--howm-bg-surface, #232733)',
};

const backStyle: React.CSSProperties = {
  color: 'var(--howm-accent, #6c8cff)',
  textDecoration: 'none',
  fontSize: '1.2rem',
  fontWeight: 600,
};

const headerNameStyle: React.CSSProperties = {
  color: 'var(--howm-text-primary, #e2e4e9)',
  fontWeight: 600,
  fontSize: '1rem',
};

const onlineDotStyle: React.CSSProperties = {
  width: '8px',
  height: '8px',
  borderRadius: '50%',
  background: '#22c55e',
};

const messagesContainerStyle: React.CSSProperties = {
  flex: 1,
  overflowY: 'auto',
  padding: '16px',
};

const mutedStyle: React.CSSProperties = {
  color: 'var(--howm-text-muted, #6b7280)',
  fontSize: '0.9rem',
  textAlign: 'center',
};

const dateDividerStyle: React.CSSProperties = {
  textAlign: 'center',
  color: 'var(--howm-text-muted, #6b7280)',
  fontSize: '0.75rem',
  margin: '12px 0 8px',
};

const bubbleBase: React.CSSProperties = {
  maxWidth: '75%',
  padding: '8px 12px',
  borderRadius: '12px',
  fontSize: '0.9rem',
  lineHeight: '1.4',
};

const sentBubbleStyle: React.CSSProperties = {
  ...bubbleBase,
  background: 'var(--howm-accent, #6c8cff)',
  color: '#fff',
  borderBottomRightRadius: '4px',
};

const receivedBubbleStyle: React.CSSProperties = {
  ...bubbleBase,
  background: 'var(--howm-bg-surface, #232733)',
  color: 'var(--howm-text-primary, #e2e4e9)',
  borderBottomLeftRadius: '4px',
};

const metaStyle: React.CSSProperties = {
  fontSize: '0.7rem',
  opacity: 0.7,
  textAlign: 'right',
  marginTop: '4px',
};

const composerContainerStyle: React.CSSProperties = {
  padding: '12px 16px',
  borderTop: '1px solid var(--howm-border, #2e3341)',
  background: 'var(--howm-bg-surface, #232733)',
};

const offlineBannerStyle: React.CSSProperties = {
  background: '#78350f',
  color: '#fcd34d',
  borderRadius: '6px',
  padding: '6px 12px',
  fontSize: '0.8rem',
  marginBottom: '8px',
  textAlign: 'center',
};

const textareaStyle: React.CSSProperties = {
  flex: 1,
  resize: 'none',
  background: 'var(--howm-bg-primary, #0f1117)',
  color: 'var(--howm-text-primary, #e2e4e9)',
  border: '1px solid var(--howm-border, #2e3341)',
  borderRadius: '8px',
  padding: '8px 12px',
  fontSize: '0.9rem',
  fontFamily: 'inherit',
  outline: 'none',
  minHeight: '36px',
  maxHeight: '120px',
};

const sendBtnStyle: React.CSSProperties = {
  background: 'var(--howm-accent, #6c8cff)',
  color: '#fff',
  border: 'none',
  borderRadius: '8px',
  padding: '8px 16px',
  fontSize: '0.9rem',
  fontWeight: 600,
  cursor: 'pointer',
  whiteSpace: 'nowrap',
};

const counterStyle: React.CSSProperties = {
  fontSize: '0.7rem',
  textAlign: 'right',
  marginTop: '4px',
};
