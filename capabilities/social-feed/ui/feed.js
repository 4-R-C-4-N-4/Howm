'use strict';

// ── State ──────────────────────────────────────────────────────────────────────
let apiToken = null;
let trustFilter = null;

// ── Initialise ─────────────────────────────────────────────────────────────────
(function init() {
  // 1. Try URL param first (simplest path — shell passes ?token=...)
  const params = new URLSearchParams(window.location.search);
  const tokenParam = params.get('token');
  if (tokenParam) {
    apiToken = tokenParam;
    loadFeed();
  } else {
    // 2. Ask parent shell via postMessage
    window.parent.postMessage({ type: 'howm:token:request' }, '*');
  }

  window.addEventListener('message', function (e) {
    if (e.origin !== window.location.origin) return;
    if (e.data && e.data.type === 'howm:token:reply') {
      apiToken = e.data.payload && e.data.payload.token;
      loadFeed();
    }
  });

  // Signal to the shell that we loaded
  window.parent.postMessage({ type: 'howm:ready', payload: { name: 'social.feed' } }, '*');

  // Auto-refresh every 30s
  setInterval(loadFeed, 30000);
})();

// ── Feed loading ───────────────────────────────────────────────────────────────
async function loadFeed() {
  var list = document.getElementById('feed-list');
  var errDiv = document.getElementById('feed-errors');
  errDiv.innerHTML = '';

  try {
    var url = trustFilter ? '/network/feed?trust=' + trustFilter : '/network/feed';
    var resp = await fetch(url);
    if (!resp.ok) throw new Error('HTTP ' + resp.status);
    var data = await resp.json();

    if (data.errors && data.errors.length > 0) {
      errDiv.innerHTML = data.errors
        .map(function (e) { return '<div class="error-banner">Peer unreachable: ' + escHtml(e) + '</div>'; })
        .join('');
    }

    var posts = data.posts || [];
    if (posts.length === 0) {
      list.innerHTML = '<p class="muted">No posts yet. Be the first!</p>';
    } else {
      list.innerHTML = posts.map(renderPost).join('');
    }
  } catch (err) {
    list.innerHTML = '<p class="muted" style="color:var(--howm-error,#f87171)">Failed to load feed. ' +
      '<a href="#" onclick="loadFeed();return false;" style="color:inherit;text-underline-offset:2px">Retry</a></p>';
  }
}

function renderPost(post) {
  var date = new Date(post.timestamp * 1000).toLocaleString();
  return '<div class="post-card">' +
    '<div class="post-header">' +
    '<span class="post-author">' + escHtml(post.author_name) + '</span>' +
    '<span class="post-time">' + date + '</span>' +
    '</div>' +
    '<div class="post-body">' + escHtml(post.content) + '</div>' +
    '</div>';
}

// ── Post submission ────────────────────────────────────────────────────────────
async function submitPost() {
  var textarea = document.getElementById('post-content');
  var btn = document.getElementById('post-btn');
  var status = document.getElementById('post-status');
  var content = textarea.value.trim();
  if (!content) return;

  btn.disabled = true;
  status.textContent = 'Posting…';
  status.className = '';

  try {
    var headers = { 'Content-Type': 'application/json' };
    if (apiToken) headers['Authorization'] = 'Bearer ' + apiToken;

    var resp = await fetch('/cap/social/post', {
      method: 'POST',
      headers: headers,
      body: JSON.stringify({ content: content }),
    });
    if (!resp.ok) throw new Error('HTTP ' + resp.status);

    textarea.value = '';
    status.textContent = 'Posted!';
    status.className = 'ok';
    window.parent.postMessage({
      type: 'howm:notify',
      payload: { level: 'success', message: 'Post published' },
    }, '*');
    loadFeed();
    setTimeout(function () { status.textContent = ''; status.className = ''; }, 3000);
  } catch (err) {
    status.textContent = 'Failed to post';
    status.className = 'err';
    window.parent.postMessage({
      type: 'howm:notify',
      payload: { level: 'error', message: 'Failed to publish post' },
    }, '*');
  } finally {
    btn.disabled = false;
  }
}

// ── Trust filter ───────────────────────────────────────────────────────────────
function setFilter(value, btn) {
  trustFilter = value;
  document.querySelectorAll('.filter-btn').forEach(function (b) { b.classList.remove('active'); });
  btn.classList.add('active');
  loadFeed();
}

// ── Utilities ──────────────────────────────────────────────────────────────────
function escHtml(str) {
  return String(str)
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');
}
