'use strict';

// ── State ──────────────────────────────────────────────────────────────────────
var apiToken=***
var trustFilter = null;
var pendingFiles = []; // files queued for upload
var mediaLimits = null; // fetched from /post/limits

// ── Base path detection ────────────────────────────────────────────────────────
// When served through the daemon proxy: /cap/social/ui/ → base = /cap/social
// When accessed directly on capability port: /ui/ → base = ''
var BASE = (function () {
  var path = window.location.pathname; // e.g. /cap/social/ui/ or /ui/
  var uiIdx = path.indexOf('/ui');
  return uiIdx > 0 ? path.substring(0, uiIdx) : '';
})();

// ── Initialise ─────────────────────────────────────────────────────────────────
(function init() {
  // 1. Try URL param first (simplest path — shell passes ?token=***
  var params = new URLSearchParams(window.location.search);
  var tokenParam=params...n');
  if (tokenParam) {
    apiToken=***
    startup();
  } else {
    // 2. Ask parent shell via postMessage
    window.parent.postMessage({ type: 'howm:token:request' }, '*');
    // Start anyway after 500ms if no reply
    setTimeout(function () { if (!apiToken) startup(); }, 500);
  }

  window.addEventListener('message', function (e) {
    if (e.origin !== window.location.origin) return;
    if (e.data && e.data.type === 'howm:token:reply') {
      apiToken=*** && e.data.payload.token;
      startup();
    }
  });

  // Signal to the shell that we loaded
  window.parent.postMessage({ type: 'howm:ready', payload: { name: 'social.feed' } }, '*');

  // File input change handler
  document.getElementById('file-input').addEventListener('change', onFilesSelected);
})();

function startup() {
  fetchLimits();
  loadFeed();
  setInterval(loadFeed, 30000);
}

// ── Limits ──────────────────────────────────────────────────────────────────────
async function fetchLimits() {
  try {
    var resp = await fetch(BASE + '/post/limits');
    if (resp.ok) {
      var data = await resp.json();
      mediaLimits = data.limits;
    }
  } catch (e) { /* non-fatal */ }
}

// ── File attachment handling ────────────────────────────────────────────────────
function onFilesSelected(e) {
  var files = Array.from(e.target.files);
  var maxCount = mediaLimits ? mediaLimits.max_attachments : 4;

  if (pendingFiles.length + files.length > maxCount) {
    alert('Max ' + maxCount + ' attachments per post');
    e.target.value = '';
    return;
  }

  // Client-side validation
  var allowed = mediaLimits ? mediaLimits.allowed_mime_types : [
    'image/jpeg', 'image/png', 'image/webp', 'image/gif', 'video/mp4', 'video/webm'
  ];
  var maxImage = mediaLimits ? mediaLimits.max_image_bytes : 8388608;
  var maxVideo = mediaLimits ? mediaLimits.max_video_bytes : 52428800;

  for (var i = 0; i < files.length; i++) {
    var f = files[i];
    if (allowed.indexOf(f.type) === -1) {
      alert('Unsupported file type: ' + f.type);
      e.target.value = '';
      return;
    }
    var isVideo = f.type.startsWith('video/');
    var limit = isVideo ? maxVideo : maxImage;
    if (f.size > limit) {
      alert(f.name + ' is too large (' + formatSize(f.size) + ', max ' + formatSize(limit) + ')');
      e.target.value = '';
      return;
    }
    pendingFiles.push(f);
  }

  e.target.value = ''; // reset so same file can be re-selected
  renderAttachmentPreview();
}

function removeAttachment(idx) {
  pendingFiles.splice(idx, 1);
  renderAttachmentPreview();
}

function renderAttachmentPreview() {
  var container = document.getElementById('attachment-preview');
  if (pendingFiles.length === 0) {
    container.innerHTML = '';
    return;
  }

  var html = '';
  for (var i = 0; i < pendingFiles.length; i++) {
    var f = pendingFiles[i];
    var isVideo = f.type.startsWith('video/');
    var preview = '';

    if (isVideo) {
      preview = '<div class="attach-thumb video-thumb">🎬</div>';
    } else {
      preview = '<img class="attach-thumb" src="' + URL.createObjectURL(f) + '" />';
    }

    html += '<div class="attach-item">' +
      preview +
      '<button class="attach-remove" onclick="removeAttachment(' + i + ')">✕</button>' +
      '<span class="attach-name">' + escHtml(f.name) + '</span>' +
      '</div>';
  }
  container.innerHTML = html;
}

// ── Feed loading ───────────────────────────────────────────────────────────────
async function loadFeed() {
  var list = document.getElementById('feed-list');
  var errDiv = document.getElementById('feed-errors');
  errDiv.innerHTML = '';

  try {
    var url = trustFilter ? BASE + '/feed?trust=' + trustFilter : BASE + '/feed';
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
      // Start polling for any peer posts with attachments
      posts.forEach(function (post) {
        if (post.attachments && post.attachments.length > 0 && post.origin !== 'local') {
          pollAttachmentStatus(post.id);
        }
      });
    }
  } catch (err) {
    list.innerHTML = '<p class="muted" style="color:var(--howm-error,#f87171)">Failed to load feed. ' +
      '<a href="#" onclick="loadFeed();return false;" style="color:inherit;text-underline-offset:2px">Retry</a></p>';
  }
}

function renderPost(post) {
  var date = new Date(post.timestamp * 1000).toLocaleString();
  var mediaHtml = renderAttachments(post);
  var deleteBtn = post.origin === 'local'
    ? '<button class="post-delete" onclick="deletePost(\'' + escAttr(post.id) + '\')" title="Delete">✕</button>'
    : '';

  return '<div class="post-card" id="post-' + escAttr(post.id) + '">' +
    '<div class="post-header">' +
    '<span class="post-author">' + escHtml(post.author_name) + '</span>' +
    '<div class="post-header-right">' +
    '<span class="post-time">' + date + '</span>' +
    deleteBtn +
    '</div>' +
    '</div>' +
    (mediaHtml ? '<div class="post-media">' + mediaHtml + '</div>' : '') +
    '<div class="post-body">' + escHtml(post.content) + '</div>' +
    '</div>';
}

function renderAttachments(post) {
  if (!post.attachments || post.attachments.length === 0) return '';

  var isPeer = post.origin && post.origin !== 'local';
  var html = '<div class="media-grid media-count-' + Math.min(post.attachments.length, 4) + '">';

  for (var i = 0; i < post.attachments.length; i++) {
    var att = post.attachments[i];
    var blobUrl = BASE + '/blob/' + att.blob_id;
    var isVideo = att.mime_type.startsWith('video/');

    if (isPeer) {
      // For peer posts, show placeholder with status indicator
      html += '<div class="media-item" id="media-' + att.blob_id + '">';
      html += '<div class="media-loading">';
      html += '<div class="media-spinner"></div>';
      html += '<span class="media-status-text">Downloading…</span>';
      html += '</div>';
      html += '</div>';
    } else if (isVideo) {
      html += '<div class="media-item">';
      html += '<video controls preload="metadata" muted>';
      html += '<source src="' + blobUrl + '" type="' + escAttr(att.mime_type) + '" />';
      html += '</video>';
      html += '</div>';
    } else {
      html += '<div class="media-item">';
      html += '<img src="' + blobUrl + '" alt="attachment" loading="lazy" onclick="openLightbox(this.src)" />';
      html += '</div>';
    }
  }

  html += '</div>';
  return html;
}

// ── Attachment status polling (peer posts) ──────────────────────────────────────
var pollingPosts = {}; // post_id -> interval

function pollAttachmentStatus(postId) {
  if (pollingPosts[postId]) return; // already polling

  var interval = setInterval(async function () {
    try {
      var resp = await fetch(BASE + '/post/' + postId + '/attachments');
      if (!resp.ok) { clearInterval(interval); delete pollingPosts[postId]; return; }
      var data = await resp.json();

      if (data.status === 'local') {
        // Local post, no transfers needed
        clearInterval(interval);
        delete pollingPosts[postId];
        return;
      }

      // Update individual attachment UIs
      var atts = data.attachments || [];
      for (var i = 0; i < atts.length; i++) {
        var t = atts[i];
        var el = document.getElementById('media-' + t.blob_id);
        if (!el) continue;

        if (t.status === 'complete') {
          // Replace placeholder with actual media
          var blobUrl = BASE + '/blob/' + t.blob_id;
          var isVideo = t.mime_type.startsWith('video/');
          if (isVideo) {
            el.innerHTML = '<video controls preload="metadata" muted>' +
              '<source src="' + blobUrl + '" type="' + escAttr(t.mime_type) + '" />' +
              '</video>';
          } else {
            el.innerHTML = '<img src="' + blobUrl + '" alt="attachment" loading="lazy" onclick="openLightbox(this.src)" />';
          }
        } else if (t.status === 'failed') {
          el.innerHTML = '<div class="media-unavailable">' +
            '<span>⚠️ Media unavailable</span>' +
            '<span class="media-hint">Peer may be offline</span>' +
            '</div>';
        } else if (t.status === 'fetching' && t.total_size > 0) {
          var pct = Math.round((t.bytes_received / t.total_size) * 100);
          var statusEl = el.querySelector('.media-status-text');
          if (statusEl) statusEl.textContent = pct + '% (' + formatSize(t.bytes_received) + ')';
        }
      }

      // Stop polling when all done
      if (data.status === 'complete' || data.status === 'partial') {
        // partial means some failed — stop polling but keep showing status
        clearInterval(interval);
        delete pollingPosts[postId];
      }
    } catch (e) {
      // Transient error, keep polling
    }
  }, 3000);

  pollingPosts[postId] = interval;
}

// ── Post submission ────────────────────────────────────────────────────────────
async function submitPost() {
  var textarea = document.getElementById('post-content');
  var btn = document.getElementById('post-btn');
  var status = document.getElementById('post-status');
  var content = textarea.value.trim();
  if (!content && pendingFiles.length === 0) return;

  btn.disabled = true;
  status.textContent = 'Posting…';
  status.className = '';

  try {
    var headers = {};
    if (apiToken) headers['Authorization'] = 'Bearer ' + apiToken;
    var resp;

    if (pendingFiles.length > 0) {
      // Multipart upload
      var form = new FormData();
      form.append('content', content);
      for (var i = 0; i < pendingFiles.length; i++) {
        form.append('file', pendingFiles[i]);
      }
      resp = await fetch(BASE + '/post/upload', {
        method: 'POST',
        headers: headers,
        body: form,
      });
    } else {
      // JSON text-only
      headers['Content-Type'] = 'application/json';
      resp = await fetch(BASE + '/post', {
        method: 'POST',
        headers: headers,
        body: JSON.stringify({ content: content }),
      });
    }

    if (!resp.ok) {
      var errData = await resp.json().catch(function () { return {}; });
      throw new Error(errData.error || 'HTTP ' + resp.status);
    }

    textarea.value = '';
    pendingFiles = [];
    renderAttachmentPreview();
    status.textContent = 'Posted!';
    status.className = 'ok';
    window.parent.postMessage({
      type: 'howm:notify',
      payload: { level: 'success', message: 'Post published' },
    }, '*');
    loadFeed();
    setTimeout(function () { status.textContent = ''; status.className = ''; }, 3000);
  } catch (err) {
    status.textContent = err.message || 'Failed to post';
    status.className = 'err';
    window.parent.postMessage({
      type: 'howm:notify',
      payload: { level: 'error', message: 'Failed to publish post' },
    }, '*');
  } finally {
    btn.disabled = false;
  }
}

// ── Post deletion ──────────────────────────────────────────────────────────────
async function deletePost(postId) {
  if (!confirm('Delete this post?')) return;
  try {
    var headers = {};
    if (apiToken) headers['Authorization'] = 'Bearer ' + apiToken;
    var resp = await fetch(BASE + '/post/' + postId, {
      method: 'DELETE',
      headers: headers,
    });
    if (!resp.ok) throw new Error('HTTP ' + resp.status);
    loadFeed();
  } catch (err) {
    alert('Failed to delete post');
  }
}

// ── Lightbox ───────────────────────────────────────────────────────────────────
function openLightbox(src) {
  var overlay = document.createElement('div');
  overlay.className = 'lightbox-overlay';
  overlay.onclick = function () { document.body.removeChild(overlay); };
  overlay.innerHTML = '<img src="' + src + '" class="lightbox-img" />';
  document.body.appendChild(overlay);
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

function escAttr(str) {
  return escHtml(str).replace(/'/g, '&#39;');
}

function formatSize(bytes) {
  if (bytes < 1024) return bytes + ' B';
  if (bytes < 1048576) return (bytes / 1024).toFixed(1) + ' KB';
  return (bytes / 1048576).toFixed(1) + ' MB';
}
