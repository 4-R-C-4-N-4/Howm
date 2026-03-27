/* Presence capability — embedded UI */

const BASE = (() => {
  // When loaded inside the howm proxy, paths are relative to /cap/presence/
  // Detect if we're in an iframe with a /cap/ prefix
  const path = window.location.pathname;
  const match = path.match(/^(\/cap\/[^/]+)/);
  return match ? match[1] : '';
})();

let token = null;

// Token handshake with parent frame
window.addEventListener('message', (e) => {
  if (e.data && e.data.type === 'howm:token:reply') {
    token = e.data.token;
    // Initial load once we have the token
    loadStatus();
    loadPeers();
  }
});

// Request token from parent
if (window.parent !== window) {
  window.parent.postMessage({ type: 'howm:token:request' }, '*');
} else {
  // Running standalone (not in iframe)
  loadStatus();
  loadPeers();
}

function headers() {
  const h = { 'Content-Type': 'application/json' };
  if (token) h['Authorization'] = 'Bearer ' + token;
  return h;
}

async function loadStatus() {
  try {
    const res = await fetch(BASE + '/status', { headers: headers() });
    if (!res.ok) return;
    const data = await res.json();

    const dot = document.getElementById('my-dot');
    dot.className = 'dot ' + data.activity;

    const emojiEl = document.getElementById('my-emoji');
    emojiEl.textContent = data.emoji || '';

    const statusEl = document.getElementById('my-status-text');
    if (data.status) {
      statusEl.textContent = data.status;
      statusEl.classList.remove('muted');
    } else {
      statusEl.textContent = 'No status set';
      statusEl.classList.add('muted');
    }

    // Pre-fill form
    document.getElementById('emoji-input').value = data.emoji || '';
    document.getElementById('status-input').value = data.status || '';
  } catch (e) {
    console.error('Failed to load status:', e);
  }
}

async function saveStatus() {
  const status = document.getElementById('status-input').value.trim() || null;
  const emoji = document.getElementById('emoji-input').value.trim() || null;

  try {
    const res = await fetch(BASE + '/status', {
      method: 'PUT',
      headers: headers(),
      body: JSON.stringify({ status, emoji }),
    });
    if (res.ok) {
      loadStatus();
    }
  } catch (e) {
    console.error('Failed to save status:', e);
  }
}

async function clearStatus() {
  document.getElementById('status-input').value = '';
  document.getElementById('emoji-input').value = '';

  try {
    await fetch(BASE + '/status', {
      method: 'PUT',
      headers: headers(),
      body: JSON.stringify({ status: null, emoji: null }),
    });
    loadStatus();
  } catch (e) {
    console.error('Failed to clear status:', e);
  }
}

async function loadPeers() {
  try {
    const res = await fetch(BASE + '/peers', { headers: headers() });
    if (!res.ok) return;
    const data = await res.json();
    renderPeers(data.peers || []);
  } catch (e) {
    console.error('Failed to load peers:', e);
  }
}

function renderPeers(peers) {
  const container = document.getElementById('peers-list');

  if (peers.length === 0) {
    container.innerHTML = '<p class="muted">No peers connected.</p>';
    return;
  }

  // Sort: active first, then away, then offline
  const order = { active: 0, away: 1, offline: 2 };
  peers.sort((a, b) => (order[a.activity] ?? 2) - (order[b.activity] ?? 2));

  container.innerHTML = peers.map(p => {
    const shortId = p.peer_id.length > 16
      ? p.peer_id.slice(0, 8) + '…' + p.peer_id.slice(-4)
      : p.peer_id;
    const emoji = p.emoji ? `<span class="peer-emoji">${esc(p.emoji)}</span>` : '';
    const status = p.status ? `<span class="peer-status">${esc(p.status)}</span>` : '';

    return `<div class="peer-row">
      <span class="dot ${esc(p.activity)}"></span>
      ${emoji}
      <span class="peer-id" title="${esc(p.peer_id)}">${esc(shortId)}</span>
      ${status}
    </div>`;
  }).join('');
}

function esc(s) {
  if (!s) return '';
  const d = document.createElement('div');
  d.textContent = s;
  return d.innerHTML;
}

// Auto-refresh peers every 5 seconds
setInterval(loadPeers, 5000);
// Refresh own status every 10 seconds
setInterval(loadStatus, 10000);
