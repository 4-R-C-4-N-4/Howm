// Howm Messaging — Embedded UI
(function () {
  'use strict';

  const BASE = '';
  let currentPeerId = null;
  let pollTimer = null;

  // ── API helpers ──────────────────────────────────────────────────────────────

  async function api(path, opts = {}) {
    const res = await fetch(BASE + path, {
      headers: { 'Content-Type': 'application/json', ...opts.headers },
      ...opts,
    });
    if (!res.ok && res.status !== 204) throw new Error(`HTTP ${res.status}`);
    if (res.status === 204) return null;
    return res.json();
  }

  // ── Conversations list ──────────────────────────────────────────────────────

  async function loadConversations() {
    const list = document.getElementById('conversation-list');
    const empty = document.getElementById('no-conversations');

    try {
      const convos = await api('/conversations');
      if (!convos || convos.length === 0) {
        list.innerHTML = '';
        empty.style.display = 'block';
        return;
      }
      empty.style.display = 'none';
      list.innerHTML = convos.map(c => {
        const last = c.last_message;
        const preview = last ? last.body_preview : '';
        const time = last ? formatTime(last.sent_at) : '';
        const active = c.peer_id === currentPeerId ? ' active' : '';
        const unread = c.unread_count > 0
          ? `<span class="conv-unread">${c.unread_count}</span>`
          : '';
        return `<li class="${active}" data-peer="${c.peer_id}" onclick="selectConversation('${c.peer_id}')">
          <div><span class="conv-peer">${shortId(c.peer_id)}</span>${unread}<span class="conv-time">${time}</span></div>
          <div class="conv-preview">${escapeHtml(preview)}</div>
        </li>`;
      }).join('');
    } catch (e) {
      console.error('Failed to load conversations:', e);
    }
  }

  // ── Chat view ───────────────────────────────────────────────────────────────

  window.selectConversation = async function (peerId) {
    currentPeerId = peerId;

    document.getElementById('select-prompt').style.display = 'none';
    document.getElementById('chat-header').style.display = 'block';
    document.getElementById('send-form').style.display = 'flex';
    document.getElementById('chat-peer-name').textContent = shortId(peerId);

    // Highlight active conversation
    document.querySelectorAll('#conversation-list li').forEach(li => {
      li.classList.toggle('active', li.dataset.peer === peerId);
    });

    await loadMessages();
    await api(`/conversations/${encodeURIComponent(peerId)}/read`, { method: 'POST' });
    await loadConversations(); // refresh unread counts

    // Start polling for new messages
    if (pollTimer) clearInterval(pollTimer);
    pollTimer = setInterval(loadMessages, 3000);
  };

  async function loadMessages() {
    if (!currentPeerId) return;
    const container = document.getElementById('messages');
    const wasAtBottom = container.scrollHeight - container.scrollTop <= container.clientHeight + 50;

    try {
      const data = await api(`/conversations/${encodeURIComponent(currentPeerId)}?limit=100`);
      if (!data || !data.messages) return;

      container.innerHTML = data.messages.map(m => {
        const cls = m.direction === 'sent' ? 'sent' : 'received';
        const time = formatTime(m.sent_at);
        const status = m.direction === 'sent'
          ? ` <span class="status ${m.delivery_status}">${m.delivery_status}</span>`
          : '';
        return `<div class="message ${cls}">
          <div>${escapeHtml(m.body)}</div>
          <div class="meta">${time}${status}</div>
        </div>`;
      }).join('');

      if (wasAtBottom) {
        container.scrollTop = container.scrollHeight;
      }
    } catch (e) {
      console.error('Failed to load messages:', e);
    }
  }

  // ── Send message ────────────────────────────────────────────────────────────

  document.getElementById('send-form').addEventListener('submit', async (e) => {
    e.preventDefault();
    if (!currentPeerId) return;

    const input = document.getElementById('msg-input');
    const body = input.value.trim();
    if (!body) return;

    input.value = '';
    input.focus();

    try {
      await api('/send', {
        method: 'POST',
        body: JSON.stringify({ to: currentPeerId, body }),
      });
      await loadMessages();
      await loadConversations();
    } catch (err) {
      console.error('Send failed:', err);
      // Show error inline
      const container = document.getElementById('messages');
      container.innerHTML += `<div class="message sent">
        <div>${escapeHtml(body)}</div>
        <div class="meta">now <span class="status failed">send failed</span></div>
      </div>`;
    }
  });

  // ── Helpers ─────────────────────────────────────────────────────────────────

  function shortId(b64) {
    return b64.length > 12 ? b64.slice(0, 8) + '…' : b64;
  }

  function formatTime(ms) {
    if (!ms) return '';
    const d = new Date(typeof ms === 'number' && ms < 1e12 ? ms * 1000 : ms);
    const now = new Date();
    if (d.toDateString() === now.toDateString()) {
      return d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
    }
    return d.toLocaleDateString([], { month: 'short', day: 'numeric' });
  }

  function escapeHtml(s) {
    if (!s) return '';
    return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
  }

  // ── Init ────────────────────────────────────────────────────────────────────

  loadConversations();
  setInterval(loadConversations, 10000);
})();
