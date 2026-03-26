'use strict';

// ── State ────────────────────────────────────────────────────────────────────
var apiToken = null;
var selectedFile = null;
var activePeers = [];
var selectedPeerId = null;
var catalogueCursor = 0;
var _started = false;
var catalogueItems = [];
var downloads = [];
var downloadPollTimer = null;
var peerPollTimer = null;


// ── Toast notifications (replaces alert()) ──────────────────────────────────

function showToast(message, level) {
  level = level || 'error';
  var container = document.getElementById('toast-container');
  var el = document.createElement('div');
  el.className = 'toast toast-' + level;
  el.textContent = message;
  container.appendChild(el);
  // Also notify the parent shell
  window.parent.postMessage({
    type: 'howm:notify',
    payload: { level: level, message: message },
  }, window.location.origin);
  setTimeout(function () {
    el.classList.add('toast-out');
    setTimeout(function () { el.remove(); }, 200);
  }, 4000);
}

// ── Inline confirm (no overlays/popups — works inside iframes) ──────────────

function inlineConfirm(triggerBtn, message, onConfirm) {
  var card = triggerBtn.closest('.offering-card');
  if (!card) return;
  var actions = card.querySelector('.offering-actions');
  if (!actions) return;
  var original = actions.innerHTML;
  actions.innerHTML =
    '<span class="inline-confirm-msg">' + escHtml(message) + '</span>' +
    '<button class="btn-delete">Yes</button>' +
    '<button class="secondary">Cancel</button>';
  actions.querySelector('.btn-delete').onclick = function () {
    actions.innerHTML = original;
    onConfirm();
  };
  actions.querySelector('.secondary').onclick = function () {
    actions.innerHTML = original;
  };
}

// ── Base path detection ──────────────────────────────────────────────────────
var BASE = (function () {
  var path = window.location.pathname;
  var uiIdx = path.indexOf('/ui');
  return uiIdx > 0 ? path.substring(0, uiIdx) : '';
})();

// ── Max file size: 500 MB ────────────────────────────────────────────────────
var MAX_FILE_SIZE = 500 * 1024 * 1024;

// ── Init ─────────────────────────────────────────────────────────────────────
// Token is delivered exclusively via postMessage from the parent shell.
// NEVER placed in URLs (leaks via Referer headers, browser history, server logs).
(function init() {
  function startOnce() { if (!_started) { _started = true; startup(); } }

  // Ask the parent shell for the token
  window.parent.postMessage({ type: 'howm:token:request' }, window.location.origin);
  // Start without auth after 500ms if no reply (read-only mode still works
  // since the daemon proxy gates by IP, not bearer token for /cap/* routes)
  setTimeout(startOnce, 500);

  window.addEventListener('message', function (e) {
    if (e.origin !== window.location.origin) return;
    if (e.data && e.data.type === 'howm:token:reply') {
      apiToken = e.data && e.data.payload && e.data.payload.token;
      startOnce();
    }
  });

  // Derive name from proxy path (/cap/{name}/ui/) instead of hardcoding
  var _capName = (window.location.pathname.match(/^\/cap\/([^/]+)/) || [])[1] || 'files';
  window.parent.postMessage({ type: 'howm:ready', payload: { name: _capName } }, window.location.origin);

  document.getElementById('file-input').addEventListener('change', onFileSelected);
  document.getElementById('upload-access').addEventListener('change', onAccessChange);
})();

function startup() {
  loadOfferings();
  loadPeers();
  loadDownloads();
  peerPollTimer = setInterval(loadPeers, 30000);
}

// ── Helpers ──────────────────────────────────────────────────────────────────

function authHeaders() {
  var h = {};
  if (apiToken) h['Authorization'] = 'Bearer ' + apiToken;
  return h;
}

function humanSize(bytes) {
  if (bytes < 1024) return bytes + ' B';
  if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(1) + ' KB';
  if (bytes < 1024 * 1024 * 1024) return (bytes / (1024 * 1024)).toFixed(1) + ' MB';
  return (bytes / (1024 * 1024 * 1024)).toFixed(2) + ' GB';
}

function timeAgo(ts) {
  var secs = Math.floor((Date.now() / 1000) - ts);
  if (secs < 60) return 'just now';
  if (secs < 3600) return Math.floor(secs / 60) + 'm ago';
  if (secs < 86400) return Math.floor(secs / 3600) + 'h ago';
  return Math.floor(secs / 86400) + 'd ago';
}

function mimeIcon(mime) {
  if (!mime) return '📁';
  if (mime.startsWith('image/')) return '🖼️';
  if (mime.startsWith('video/')) return '🎬';
  if (mime.startsWith('audio/')) return '🎵';
  if (mime.includes('pdf')) return '📄';
  if (mime.includes('zip') || mime.includes('tar') || mime.includes('gzip') || mime.includes('compress'))
    return '📦';
  if (mime.includes('text')) return '📝';
  return '📁';
}

function accessBadge(access) {
  var cls = 'badge ';
  if (access === 'public') cls += 'badge-public';
  else if (access === 'friends') cls += 'badge-friends';
  else if (access === 'trusted') cls += 'badge-trusted';
  else cls += 'badge-peer';
  return '<span class="' + cls + '">' + escHtml(access) + '</span>';
}

function escHtml(s) {
  var d = document.createElement('div');
  d.textContent = s;
  return d.innerHTML;
}

function shortId(id) {
  return id.length > 12 ? id.substring(0, 8) + '…' : id;
}

// ── Tabs ─────────────────────────────────────────────────────────────────────

function switchTab(name, btn) {
  document.querySelectorAll('.tab').forEach(function (t) { t.classList.remove('active'); });
  btn.classList.add('active');
  document.getElementById('tab-my-files').classList.toggle('hidden', name !== 'my-files');
  document.getElementById('tab-browse').classList.toggle('hidden', name !== 'browse');
  document.getElementById('tab-downloads').classList.toggle('hidden', name !== 'downloads');

  if (name === 'downloads') loadDownloads();
  if (name === 'browse') loadPeers();
}

// ── Upload / drag-drop ───────────────────────────────────────────────────────

function onDragOver(e) {
  e.preventDefault();
  document.getElementById('drop-area').classList.add('dragover');
}
function onDragLeave(e) {
  document.getElementById('drop-area').classList.remove('dragover');
}
function onDrop(e) {
  e.preventDefault();
  document.getElementById('drop-area').classList.remove('dragover');
  if (e.dataTransfer.files.length > 0) pickFile(e.dataTransfer.files[0]);
}
function onFileSelected() {
  var inp = document.getElementById('file-input');
  if (inp.files.length > 0) pickFile(inp.files[0]);
}

function pickFile(file) {
  if (file.size > MAX_FILE_SIZE) {
    setStatus('upload-status', 'err', 'File too large (max 500 MB)');
    return;
  }
  selectedFile = file;
  document.getElementById('drop-area').classList.add('hidden');
  document.getElementById('upload-form').classList.remove('hidden');
  document.getElementById('file-preview').textContent =
    mimeIcon(file.type) + ' ' + file.name + ' (' + humanSize(file.size) + ')';
  document.getElementById('upload-name').value = file.name;
  setStatus('upload-status', '', '');
}

function cancelUpload() {
  selectedFile = null;
  document.getElementById('upload-form').classList.add('hidden');
  document.getElementById('drop-area').classList.remove('hidden');
  document.getElementById('upload-progress').classList.add('hidden');
  document.getElementById('file-input').value = '';
  setStatus('upload-status', '', '');
}

function onAccessChange() {
  var v = document.getElementById('upload-access').value;
  document.getElementById('peer-select-row').classList.toggle('hidden', v !== 'peer');
  if (v === 'peer') populatePeerSelect();
}

function populatePeerSelect() {
  var sel = document.getElementById('upload-peers');
  sel.innerHTML = '';
  activePeers.forEach(function (p) {
    var opt = document.createElement('option');
    opt.value = p.peer_id;
    opt.textContent = shortId(p.peer_id);
    sel.appendChild(opt);
  });
}

async function submitUpload() {
  if (!selectedFile) return;
  var name = document.getElementById('upload-name').value.trim();
  if (!name) { setStatus('upload-status', 'err', 'Name required'); return; }

  var access = document.getElementById('upload-access').value;
  var form = new FormData();
  form.append('file', selectedFile);
  form.append('name', name);
  var desc = document.getElementById('upload-desc').value.trim();
  if (desc) form.append('description', desc);
  form.append('access', access);

  if (access === 'peer') {
    var sel = document.getElementById('upload-peers');
    var peers = Array.from(sel.selectedOptions).map(function (o) { return o.value; });
    if (peers.length === 0) { setStatus('upload-status', 'err', 'Select at least one peer'); return; }
    form.append('allowlist', JSON.stringify(peers));
  }

  setStatus('upload-status', '', 'Uploading…');
  document.getElementById('upload-progress').classList.remove('hidden');

  try {
    var xhr = new XMLHttpRequest();
    xhr.open('POST', BASE + '/offerings');
    if (apiToken) xhr.setRequestHeader('Authorization', 'Bearer ' + apiToken);

    xhr.upload.onprogress = function (e) {
      if (e.lengthComputable) {
        var pct = Math.round((e.loaded / e.total) * 100);
        document.getElementById('progress-fill').style.width = pct + '%';
      }
    };

    xhr.onload = function () {
      if (xhr.status >= 200 && xhr.status < 300) {
        setStatus('upload-status', 'ok', 'Uploaded!');
        cancelUpload();
        loadOfferings();
      } else {
        var msg = 'Upload failed';
        try { msg = JSON.parse(xhr.responseText).error || msg; } catch (_) {}
        setStatus('upload-status', 'err', msg);
      }
    };
    xhr.onerror = function () { setStatus('upload-status', 'err', 'Network error'); };
    xhr.send(form);
  } catch (e) {
    setStatus('upload-status', 'err', e.message);
  }
}

function setStatus(id, cls, msg) {
  var el = document.getElementById(id);
  el.className = cls;
  el.textContent = msg;
}

// ── Offerings ────────────────────────────────────────────────────────────────

async function loadOfferings() {
  try {
    var resp = await fetch(BASE + '/offerings', { headers: authHeaders() });
    if (!resp.ok) throw new Error('HTTP ' + resp.status);
    var data = await resp.json();
    renderOfferings(data.offerings || []);
  } catch (e) {
    document.getElementById('offerings-list').innerHTML =
      '<p class="muted">Failed to load offerings</p>';
  }
}

function renderOfferings(items) {
  var el = document.getElementById('offerings-list');
  if (items.length === 0) {
    el.innerHTML = '<p class="muted">No offerings yet. Upload a file above.</p>';
    document.getElementById('offerings-heading').textContent = 'Offerings';
    return;
  }
  document.getElementById('offerings-heading').textContent = 'Offerings (' + items.length + ')';

  el.innerHTML = items.map(function (o) {
    var icon = mimeIcon(o.mime_type);
    var blobShort = o.blob_id ? shortId(o.blob_id) : '';
    return '<div class="offering-card">' +
      '<div class="offering-header">' +
        '<span class="offering-name"><span class="offering-icon">' + icon + '</span>' + escHtml(o.name) + '</span>' +
        '<div class="offering-actions">' +
          '<button onclick="openEdit(\'' + escHtml(o.offering_id) + '\')">Edit</button>' +
          '<button class="btn-delete" onclick="confirmDeleteOffering(this, \'' + escHtml(o.offering_id) + '\', \'' + escHtml(o.name) + '\')">Delete</button>' +
        '</div>' +
      '</div>' +
      (o.description ? '<div class="offering-desc">' + escHtml(o.description) + '</div>' : '') +
      '<div class="offering-meta">' +
        '<span>' + humanSize(o.size || 0) + '</span>' +
        accessBadge(o.access || 'public') +
        '<span class="offering-blob" title="Click to copy" onclick="navigator.clipboard.writeText(\'' + escHtml(o.blob_id || '') + '\')">' + blobShort + '</span>' +
        '<span>' + (o.created_at ? timeAgo(o.created_at) : '') + '</span>' +
      '</div>' +
    '</div>';
  }).join('');
}

// ── Edit modal ───────────────────────────────────────────────────────────────

function openEdit(id) {
  // Find the offering by scanning DOM — or re-fetch. For simplicity, just populate fields.
  document.getElementById('edit-id').value = id;
  document.getElementById('edit-overlay').classList.remove('hidden');
}

function closeEdit() {
  document.getElementById('edit-overlay').classList.add('hidden');
}

async function saveEdit() {
  var id = document.getElementById('edit-id').value;
  var body = {};
  var name = document.getElementById('edit-name').value.trim();
  var desc = document.getElementById('edit-desc').value.trim();
  var access = document.getElementById('edit-access').value;
  if (name) body.name = name;
  body.description = desc;
  body.access = access;

  try {
    var resp = await fetch(BASE + '/offerings/' + encodeURIComponent(id), {
      method: 'PATCH',
      headers: Object.assign({ 'Content-Type': 'application/json' }, authHeaders()),
      body: JSON.stringify(body),
    });
    if (!resp.ok) {
      var msg = 'Update failed';
      try { msg = (await resp.json()).error || msg; } catch (_) {}
      showToast(msg);
      return;
    }
    closeEdit();
    loadOfferings();
    showToast('Offering updated', 'success');
  } catch (e) {
    showToast(e.message);
  }
}

// ── Delete offering ──────────────────────────────────────────────────────────

function confirmDeleteOffering(btn, id, name) {
  inlineConfirm(btn, 'Delete "' + name + '"?', function () {
    deleteOffering(id);
  });
}

async function deleteOffering(id) {
  try {
    var resp = await fetch(BASE + '/offerings/' + encodeURIComponent(id), {
      method: 'DELETE',
      headers: authHeaders(),
    });
    if (!resp.ok) {
      var msg = 'Delete failed';
      try { msg = (await resp.json()).error || msg; } catch (_) {}
      showToast(msg);
      return;
    }
    loadOfferings();
    showToast('Offering deleted', 'success');
  } catch (e) {
    showToast(e.message);
  }
}

// ── Peers ────────────────────────────────────────────────────────────────────

async function loadPeers() {
  try {
    var resp = await fetch(BASE + '/peers', { headers: authHeaders() });
    if (!resp.ok) throw new Error('HTTP ' + resp.status);
    var data = await resp.json();
    activePeers = data.peers || [];
    renderPeers();
  } catch (e) {
    document.getElementById('peer-list').innerHTML =
      '<p class="muted">Failed to load peers</p>';
  }
}

function renderPeers() {
  var el = document.getElementById('peer-list');
  if (activePeers.length === 0) {
    el.innerHTML = '<p class="muted">No peers online with files capability.</p>';
    return;
  }
  el.innerHTML = activePeers.map(function (p) {
    var sel = (selectedPeerId === p.peer_id) ? ' selected' : '';
    return '<div class="peer-card' + sel + '" onclick="selectPeer(\'' + escHtml(p.peer_id) + '\')">' +
      '<span class="peer-dot"></span>' +
      '<span class="peer-id">' + shortId(p.peer_id) + '</span>' +
      (p.wg_address ? ' <span style="opacity:0.5;font-size:0.8rem">(' + escHtml(p.wg_address) + ')</span>' : '') +
    '</div>';
  }).join('');
}

async function selectPeer(peerId) {
  selectedPeerId = peerId;
  catalogueCursor = 0;
  catalogueItems = [];
  renderPeers();

  document.getElementById('peer-catalogue').classList.remove('hidden');
  document.getElementById('catalogue-heading').textContent = shortId(peerId) + '\'s catalogue';
  document.getElementById('catalogue-list').innerHTML = '<p class="muted">Loading…</p>';
  await loadCatalogue();
}

async function loadCatalogue() {
  try {
    var url = BASE + '/peer/' + encodeURIComponent(selectedPeerId) + '/catalogue?limit=20&cursor=' + catalogueCursor;
    var resp = await fetch(url, { headers: authHeaders() });
    if (!resp.ok) throw new Error('HTTP ' + resp.status);
    var data = await resp.json();
    var items = data.offerings || [];
    catalogueItems = catalogueItems.concat(items);
    renderCatalogue();

    var more = document.getElementById('catalogue-more');
    if (data.next_cursor !== undefined && data.next_cursor !== null && items.length >= 20) {
      catalogueCursor = data.next_cursor;
      more.classList.remove('hidden');
    } else {
      more.classList.add('hidden');
    }
  } catch (e) {
    document.getElementById('catalogue-list').innerHTML =
      '<p class="muted">Failed to load catalogue: ' + escHtml(e.message) + '</p>';
  }
}

function loadMoreCatalogue() { loadCatalogue(); }

function renderCatalogue() {
  var el = document.getElementById('catalogue-list');
  if (catalogueItems.length === 0) {
    el.innerHTML = '<p class="muted">No files shared by this peer.</p>';
    return;
  }
  el.innerHTML = catalogueItems.map(function (o) {
    var icon = mimeIcon(o.mime_type);
    return '<div class="cat-item">' +
      '<div class="cat-info">' +
        '<span class="cat-name">' + icon + ' ' + escHtml(o.name) + '</span>' +
        (o.description ? '<div class="cat-desc">' + escHtml(o.description) + '</div>' : '') +
        '<span class="cat-size">' + humanSize(o.size || 0) + '</span>' +
      '</div>' +
      '<button onclick="initiateDownload(\'' + escHtml(selectedPeerId) + '\', ' + JSON.stringify(JSON.stringify(o)) + ')">Download</button>' +
    '</div>';
  }).join('');
}

// ── Downloads ────────────────────────────────────────────────────────────────

async function initiateDownload(peerId, offeringJson) {
  var o = JSON.parse(offeringJson);
  try {
    var resp = await fetch(BASE + '/downloads', {
      method: 'POST',
      headers: Object.assign({ 'Content-Type': 'application/json' }, authHeaders()),
      body: JSON.stringify({
        peer_id: peerId,
        offering_id: o.offering_id,
        blob_id: o.blob_id,
        name: o.name,
        mime_type: o.mime_type || 'application/octet-stream',
        size: o.size || 0,
      }),
    });
    if (!resp.ok) {
      var msg = 'Download failed';
      try { msg = (await resp.json()).error || msg; } catch (_) {}
      showToast(msg);
      return;
    }
    // Switch to downloads tab
    switchTab('downloads', document.getElementById('downloads-tab'));
    loadDownloads();
    startDownloadPolling();
    showToast('Download started', 'info');
  } catch (e) {
    showToast(e.message);
  }
}

async function loadDownloads() {
  try {
    var resp = await fetch(BASE + '/downloads', { headers: authHeaders() });
    if (!resp.ok) throw new Error('HTTP ' + resp.status);
    var data = await resp.json();
    downloads = data.downloads || [];
    renderDownloads();

    // Start/stop polling based on active downloads
    var hasActive = downloads.some(function (d) {
      return d.status === 'pending' || d.status === 'transferring';
    });
    if (hasActive) startDownloadPolling();
    else stopDownloadPolling();
  } catch (e) {
    document.getElementById('downloads-list').innerHTML =
      '<p class="muted">Failed to load downloads</p>';
  }
}

function startDownloadPolling() {
  if (downloadPollTimer) return;
  downloadPollTimer = setInterval(loadDownloads, 3000);
}

function stopDownloadPolling() {
  if (downloadPollTimer) {
    clearInterval(downloadPollTimer);
    downloadPollTimer = null;
  }
}

function renderDownloads() {
  var el = document.getElementById('downloads-list');
  if (downloads.length === 0) {
    el.innerHTML = '<p class="muted">No downloads yet.</p>';
    return;
  }

  var activeCount = downloads.filter(function (d) {
    return d.status === 'pending' || d.status === 'transferring';
  }).length;
  var completeCount = downloads.filter(function (d) { return d.status === 'complete'; }).length;

  el.innerHTML = downloads.map(function (d) {
    var icon, statusText, action;
    switch (d.status) {
      case 'pending':
        icon = '⏳'; statusText = 'pending'; action = ''; break;
      case 'transferring':
        icon = '⟳'; statusText = 'transferring'; action = ''; break;
      case 'complete':
        icon = '✓'; statusText = 'complete';
        action = ' <a href="' + BASE + '/downloads/' + encodeURIComponent(d.blob_id) + '/data" target="_blank"><button>Save</button></a>';
        break;
      case 'failed':
        icon = '✗'; statusText = 'failed';
        action = ' <button class="secondary" onclick="retryDownload(\'' + escHtml(d.blob_id) + '\')">Retry</button>';
        break;
      default:
        icon = '?'; statusText = d.status; action = '';
    }
    return '<div class="dl-item">' +
      '<div>' +
        '<span class="dl-status-icon">' + icon + '</span>' +
        '<span class="dl-name">' + escHtml(d.name) + '</span>' +
        '<span class="dl-size">' + humanSize(d.size || 0) + '</span>' +
        '<span class="dl-status">' + statusText + '</span>' +
      '</div>' +
      '<div>' + action + '</div>' +
    '</div>';
  }).join('');
}

async function retryDownload(blobId) {
  var dl = downloads.find(function (d) { return d.blob_id === blobId; });
  if (!dl) return;
  try {
    await fetch(BASE + '/downloads', {
      method: 'POST',
      headers: Object.assign({ 'Content-Type': 'application/json' }, authHeaders()),
      body: JSON.stringify({
        peer_id: dl.peer_id,
        offering_id: dl.offering_id,
        blob_id: dl.blob_id,
        name: dl.name,
        mime_type: dl.mime_type || 'application/octet-stream',
        size: dl.size || 0,
      }),
    });
    loadDownloads();
    startDownloadPolling();
  } catch (e) {
    showToast(e.message);
  }
}
