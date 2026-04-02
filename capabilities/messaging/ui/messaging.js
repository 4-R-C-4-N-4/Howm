"use strict";

// ── State ──────────────────────────────────────────────────────────────────────
var apiToken = null;
var currentPeerId = null;
var peers = []; // cached peer list for name resolution
var lastUnread = -1; // track badge changes
var pollTimer = null;
var convPollTimer = null;
var _started = false;
var optimisticMessages = []; // messages inserted before server confirms
var presenceMap = {}; // peer_id → { activity, status, emoji }

// ── Base path detection ────────────────────────────────────────────────────────
// When served through daemon proxy: /cap/messaging/ui/ → base = /cap/messaging
// When accessed directly on capability port: /ui/ → base = ''
var BASE = (function () {
  var path = window.location.pathname;
  var uiIdx = path.indexOf("/ui");
  return uiIdx > 0 ? path.substring(0, uiIdx) : "";
})();

// The daemon API base (for /node/peers and /notifications/*)
var DAEMON_BASE = (function () {
  // If we're behind the proxy at /cap/messaging/ui/, daemon is at /
  // If direct, daemon is at the same host but root
  return "";
})();

// ── Initialise ─────────────────────────────────────────────────────────────────

(function init() {
  function startOnce() {
    if (!_started) {
      _started = true;
      startup();
    }
  }

  // Ask parent shell for the token
  window.parent.postMessage(
    { type: "howm:token:request" },
    window.location.origin,
  );

  // Canonical name — updated from token reply if available, else fallback.
  var _capName = "messaging";
  var _readySent = false;

  function signalReady() {
    if (_readySent) return;
    _readySent = true;
    window.parent.postMessage(
      { type: "howm:ready", payload: { name: _capName } },
      window.location.origin,
    );
  }

  window.addEventListener("message", function (e) {
    if (e.origin !== window.location.origin) return;
    if (e.data && e.data.type === "howm:token:reply") {
      apiToken = e.data && e.data.payload && e.data.payload.token;
      if (e.data.payload && e.data.payload.name) _capName = e.data.payload.name;
      signalReady();
      startOnce();
    }
    // Deep link from shell
    if (e.data && e.data.type === "howm:navigate:to") {
      var params = e.data.payload && e.data.payload.params;
      if (params && params.peer) {
        window.location.hash = "#/chat/" + encodeURIComponent(params.peer);
      }
    }
  });

  // If no token reply within 500ms, signal ready with fallback name and start anyway
  setTimeout(function () {
    signalReady();
    startOnce();
  }, 500);

  // Hash routing
  window.addEventListener("hashchange", route);

  // Check for deep link query params on initial load
  var urlParams = new URLSearchParams(window.location.search);
  var peerParam = urlParams.get("peer");
  if (peerParam) {
    window.location.hash = "#/chat/" + encodeURIComponent(peerParam);
  }
})();

// ── Routing ────────────────────────────────────────────────────────────────────

function route() {
  var hash = window.location.hash || "#/";

  if (hash.indexOf("#/chat/") === 0) {
    var peerId = decodeURIComponent(hash.substring(7));
    showChatView(peerId);
  } else {
    showListView();
  }
}

function showListView() {
  currentPeerId = null;
  if (pollTimer) {
    clearInterval(pollTimer);
    pollTimer = null;
  }
  document.getElementById("list-view").style.display = "block";
  document.getElementById("chat-view").style.display = "none";
  loadConversations();
}

function showChatView(peerId) {
  currentPeerId = peerId;
  optimisticMessages = [];
  document.getElementById("list-view").style.display = "none";
  document.getElementById("chat-view").style.display = "flex";
  document.getElementById("chat-peer-name").textContent = peerName(peerId);
  // updateOnlineStatus(peerId);
  document.getElementById("msg-input").value = "";
  updateByteCounter();

  loadMessages();
  markRead(peerId);

  if (pollTimer) clearInterval(pollTimer);
  pollTimer = setInterval(function () {
    loadMessages();
  }, 3000);
}

// ── Startup ────────────────────────────────────────────────────────────────────

function startup() {
  fetchPeers();
  fetchPresence();
  route();

  // Poll conversations list every 5s (for badge updates and list refresh)
  convPollTimer = setInterval(function () {
    if (!currentPeerId) loadConversations();
    updateBadge();
  }, 5000);

  // Refresh peers every 30s
  setInterval(fetchPeers, 30000);

  // Refresh presence every 5s
  setInterval(fetchPresence, 5000);
}

function fetchPresence() {
  daemonFetch("/cap/presence/peers")
    .then(function (data) {
      if (!data || !data.peers) return;
      presenceMap = {};
      for (var i = 0; i < data.peers.length; i++) {
        var p = data.peers[i];
        presenceMap[p.peer_id] = p;
      }
    })
    .catch(function () {
      // Presence capability may not be running — ignore
    });
}

// ── API helpers ────────────────────────────────────────────────────────────────

function apiFetch(path, opts) {
  opts = opts || {};
  var headers = { "Content-Type": "application/json" };
  if (apiToken) headers["Authorization"] = "Bearer " + apiToken;
  if (opts.headers) {
    for (var k in opts.headers) headers[k] = opts.headers[k];
  }
  opts.headers = headers;
  return fetch(BASE + path, opts).then(function (res) {
    if (!res.ok && res.status !== 204) throw new Error("HTTP " + res.status);
    if (res.status === 204) return null;
    return res.json();
  });
}

function daemonFetch(path, opts) {
  opts = opts || {};
  var headers = { "Content-Type": "application/json" };
  if (apiToken) headers["Authorization"] = "Bearer " + apiToken;
  if (opts.headers) {
    for (var k in opts.headers) headers[k] = opts.headers[k];
  }
  opts.headers = headers;
  return fetch(DAEMON_BASE + path, opts).then(function (res) {
    if (!res.ok && res.status !== 204) throw new Error("HTTP " + res.status);
    if (res.status === 204) return null;
    return res.json();
  });
}

// ── Peer resolution ────────────────────────────────────────────────────────────

function fetchPeers() {
  daemonFetch("/node/peers")
    .then(function (data) {
      // Response is { peers: [...] }, not a bare array
      if (data && Array.isArray(data.peers)) {
        peers = data.peers;
        // Re-render online status if a chat is open
        if (currentPeerId) updateOnlineStatus(currentPeerId);
      }
    })
    .catch(function () {
      /* non-fatal */
    });
}

function peerName(pubkey) {
  for (var i = 0; i < peers.length; i++) {
    if (peers[i].wg_pubkey === pubkey) return peers[i].name;
  }
  return pubkey.length > 12 ? pubkey.slice(0, 12) + "…" : pubkey;
}

function isPeerOnline(pubkey) {
  var now = Date.now();
  for (var i = 0; i < peers.length; i++) {
    if (peers[i].wg_pubkey === pubkey) {
      // last_seen is a Unix timestamp in seconds; peer is online if seen within 90s
      var lastSeen = peers[i].last_seen * 1000;
      return (now - lastSeen) < 90000;
    }
  }
  return false;
}

// ── Conversations list ─────────────────────────────────────────────────────────

function loadConversations() {
  apiFetch("/conversations")
    .then(function (convos) {
      var list = document.getElementById("conversation-list");
      var empty = document.getElementById("no-conversations");

      if (!convos || convos.length === 0) {
        list.innerHTML = "";
        empty.style.display = "block";
        return;
      }
      empty.style.display = "none";

      // Sort by most recent
      convos.sort(function (a, b) {
        var at = a.last_message ? a.last_message.sent_at : 0;
        var bt = b.last_message ? b.last_message.sent_at : 0;
        return bt - at;
      });

      list.innerHTML = convos
        .map(function (c) {
          var last = c.last_message;
          var preview = last ? last.body_preview : "";
          var time = last ? formatTime(last.sent_at) : "";
          var prefix = last && last.direction === "sent" ? "You: " : "";
          var unread =
            c.unread_count > 0
              ? '<span class="conv-unread">' + c.unread_count + "</span>"
              : "";
          var name = peerName(c.peer_id);
          var encodedPeer = encodeURIComponent(c.peer_id);
          var pres = presenceMap[c.peer_id];
          var dotColor = pres
            ? pres.activity === "active"
              ? "#22c55e"
              : "#eab308"
            : "#555";
          var dot =
            '<span style="display:inline-block;width:8px;height:8px;border-radius:50%;background:' +
            dotColor +
            ';margin-right:6px;flex-shrink:0;"></span>';
          var presEmoji =
            pres && pres.emoji
              ? ' <span style="font-size:12px">' +
                escapeHtml(pres.emoji) +
                "</span>"
              : "";
          return (
            "<li onclick=\"window.location.hash='#/chat/" +
            encodedPeer +
            "'\">" +
            '<div style="display:flex;align-items:center">' +
            dot +
            '<span class="conv-peer">' +
            escapeHtml(name) +
            "</span>" +
            presEmoji +
            unread +
            '<span class="conv-time">' +
            time +
            "</span></div>" +
            '<div class="conv-preview">' +
            escapeHtml(prefix + preview) +
            "</div>" +
            "</li>"
          );
        })
        .join("");

      // Update badge
      updateBadge();
    })
    .catch(function (e) {
      console.error("Failed to load conversations:", e);
    });
}

// ── Badge push ─────────────────────────────────────────────────────────────────

function updateBadge() {
  apiFetch("/conversations")
    .then(function (convos) {
      if (!convos) return;
      var total = 0;
      for (var i = 0; i < convos.length; i++) {
        total += convos[i].unread_count || 0;
      }
      if (total !== lastUnread) {
        lastUnread = total;
        daemonFetch("/notifications/badge", {
          method: "POST",
          body: JSON.stringify({
            capability: "social.messaging",
            count: total,
          }),
        }).catch(function () {
          /* fire-and-forget */
        });
      }
    })
    .catch(function () {
      /* non-fatal */
    });
}

// ── Chat view ──────────────────────────────────────────────────────────────────

function loadMessages() {
  if (!currentPeerId) return;
  var container = document.getElementById("messages");
  var wasAtBottom =
    container.scrollHeight - container.scrollTop <= container.clientHeight + 50;
  // Update online status
  updateOnlineStatus(currentPeerId);

  apiFetch("/conversations/" + encodeURIComponent(currentPeerId) + "?limit=100")
    .then(function (data) {
      if (!data || !data.messages) return;

      var serverMsgs = data.messages;
      // Merge with optimistic messages not yet confirmed
      var serverIds = {};
      for (var i = 0; i < serverMsgs.length; i++)
        serverIds[serverMsgs[i].msg_id] = true;
      var remaining = [];
      for (var j = 0; j < optimisticMessages.length; j++) {
        if (!serverIds[optimisticMessages[j].msg_id])
          remaining.push(optimisticMessages[j]);
      }
      optimisticMessages = remaining;

      var allMessages = serverMsgs.concat(optimisticMessages);
      allMessages.sort(function (a, b) {
        return a.sent_at - b.sent_at;
      });

      var html = "";
      var lastDate = "";
      for (var k = 0; k < allMessages.length; k++) {
        var m = allMessages[k];
        var dateStr = formatDate(m.sent_at);
        if (dateStr !== lastDate) {
          html += '<div class="date-divider">' + dateStr + "</div>";
          lastDate = dateStr;
        }
        var cls = m.direction === "sent" ? "sent" : "received";
        var time = formatTimeShort(m.sent_at);
        var status = "";
        if (m.direction === "sent") {
          if (m.delivery_status === "pending")
            status = ' <span class="status pending">⏳</span>';
          else if (m.delivery_status === "delivered")
            status = ' <span class="status delivered">✓</span>';
          else if (m.delivery_status === "failed")
            status = ' <span class="status failed">⚠</span>';
        }
        html +=
          '<div class="message ' +
          cls +
          '">' +
          "<div>" +
          escapeHtml(m.body) +
          "</div>" +
          '<div class="meta">' +
          time +
          status +
          "</div>" +
          "</div>";
      }
      container.innerHTML = html;

      if (wasAtBottom) {
        container.scrollTop = container.scrollHeight;
      }
    })
    .catch(function (e) {
      console.error("Failed to load messages:", e);
    });
}

function markRead(peerId) {
  apiFetch("/conversations/" + encodeURIComponent(peerId) + "/read", {
    method: "POST",
  })
    .then(function () {
      updateBadge();
    })
    .catch(function () {
      /* non-fatal */
    });
}

function updateOnlineStatus(peerId) {
  var online = isPeerOnline(peerId);
  var banner = document.getElementById("offline-banner");
  var input = document.getElementById("msg-input");
  var sendBtn = document.getElementById("send-btn");

  if (online) {
    banner.style.display = "none";
    input.disabled = false;
    input.placeholder = "Type a message…";
    sendBtn.disabled = false;
  } else {
    banner.style.display = "block";
    input.disabled = true;
    input.placeholder = "Peer is offline";
    sendBtn.disabled = true;
  }
}

// ── Send message ───────────────────────────────────────────────────────────────

function sendMessage() {
  if (!currentPeerId) return;
  var input = document.getElementById("msg-input");
  var body = input.value.trim();
  if (!body) return;

  var byteLen = new TextEncoder().encode(body).length;
  if (byteLen > 4096) return;

  // Optimistic insert
  var tempId = "opt-" + Date.now();
  optimisticMessages.push({
    msg_id: tempId,
    conversation_id: "",
    direction: "sent",
    sender_peer_id: "",
    sent_at: Date.now(),
    body: body,
    delivery_status: "pending",
  });
  input.value = "";
  updateByteCounter();
  loadMessages(); // re-render with optimistic

  apiFetch("/send", {
    method: "POST",
    body: JSON.stringify({ to: currentPeerId, body: body }),
  })
    .then(function () {
      loadMessages();
      loadConversations();
    })
    .catch(function (err) {
      console.error("Send failed:", err);
      // Mark optimistic message as failed
      for (var i = 0; i < optimisticMessages.length; i++) {
        if (optimisticMessages[i].msg_id === tempId) {
          optimisticMessages[i].delivery_status = "failed";
        }
      }
      loadMessages();
    });
}

// ── Byte counter ───────────────────────────────────────────────────────────────

function updateByteCounter() {
  var input = document.getElementById("msg-input");
  var counter = document.getElementById("byte-counter");
  var sendBtn = document.getElementById("send-btn");
  var val = input.value || "";
  var byteLen = new TextEncoder().encode(val).length;
  counter.textContent = byteLen + " / 4096";

  if (byteLen > 4096) {
    counter.className = "byte-counter over-limit";
    sendBtn.disabled = true;
  } else if (byteLen > 4000) {
    counter.className = "byte-counter near-limit";
    sendBtn.disabled = false;
  } else {
    counter.className = "byte-counter";
    sendBtn.disabled = false;
  }
}

// ── Helpers ────────────────────────────────────────────────────────────────────

function formatTime(ms) {
  if (!ms) return "";
  var d = new Date(typeof ms === "number" && ms < 1e12 ? ms * 1000 : ms);
  var now = new Date();
  if (d.toDateString() === now.toDateString()) {
    return d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  }
  return d.toLocaleDateString([], { month: "short", day: "numeric" });
}

function formatTimeShort(ms) {
  var d = new Date(typeof ms === "number" && ms < 1e12 ? ms * 1000 : ms);
  return d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

function formatDate(ms) {
  var d = new Date(typeof ms === "number" && ms < 1e12 ? ms * 1000 : ms);
  var now = new Date();
  if (d.toDateString() === now.toDateString()) return "Today";
  var yesterday = new Date(now);
  yesterday.setDate(yesterday.getDate() - 1);
  if (d.toDateString() === yesterday.toDateString()) return "Yesterday";
  return d.toLocaleDateString([], {
    month: "short",
    day: "numeric",
    year: "numeric",
  });
}

function escapeHtml(s) {
  if (!s) return "";
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}
