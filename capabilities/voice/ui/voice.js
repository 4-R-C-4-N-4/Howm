// Voice capability UI — room management + WebRTC audio
'use strict';

// Base path detection — when loaded in the daemon proxy iframe at
// /cap/voice/ui/, API calls need to target /cap/voice/rooms etc.
const BASE = (function () {
  const path = window.location.pathname;
  const uiIdx = path.indexOf('/ui');
  return uiIdx > 0 ? path.substring(0, uiIdx) : '';
})();
let currentRoom = null;
let isMuted = false;
let ws = null;
let peerConnections = {};  // peer_id -> RTCPeerConnection
let localStream = null;
let audioAnalysers = {};   // peer_id -> { analyser, dataArray }
let levelAnimFrame = null; // requestAnimationFrame id
let availablePeers = [];   // cached from voice tracker

// ── Peer ID ──────────────────────────────────────────────────────────────────

// The real peer ID is learned from the server via GET /me (which reads the
// X-Node-Id / X-Peer-Id header injected by the daemon proxy). The WS
// handshake sends this so the signal server can verify room membership.
let PEER_ID = null;

async function fetchMyId() {
  try {
    const resp = await fetch(`${BASE}/me`, { headers: apiHeaders() });
    if (resp.ok) {
      const data = await resp.json();
      if (data.peer_id) PEER_ID = data.peer_id;
    }
  } catch (_) {}
}

function apiHeaders() {
  return { 'Content-Type': 'application/json', 'X-Peer-Id': PEER_ID };
}

function shortId(id) {
  return id && id.length > 12 ? id.slice(0, 8) + '…' : id;
}

function esc(s) {
  const d = document.createElement('div');
  d.textContent = s;
  return d.innerHTML;
}

// ── Active peer list (from voice tracker, not presence) ─────────────────────

async function loadPeers() {
  try {
    const resp = await fetch(`${BASE}/peers`, { headers: apiHeaders() });
    if (!resp.ok) return;
    const data = await resp.json();
    availablePeers = (data.peers || []).filter(p => p.peer_id !== PEER_ID);
  } catch (_) {
    availablePeers = [];
  }
}

function buildPeerOptions(excludeIds) {
  const exclude = new Set(excludeIds || []);
  return availablePeers
    .filter(p => !exclude.has(p.peer_id))
    .map(p => {
      const busy = p.status && p.status.startsWith('In a call');
      const label = `${shortId(p.peer_id)} ${p.emoji || ''} ${busy ? '(in call)' : p.activity === 'Away' ? '(away)' : ''}`.trim();
      return { peer_id: p.peer_id, label, busy, away: p.activity === 'Away' };
    });
}

function showPeerPicker(title, excludeIds) {
  return new Promise((resolve) => {
    const options = buildPeerOptions(excludeIds);
    if (!options.length) {
      // Fallback to manual input
      const input = prompt(`${title}\nNo peers from presence. Enter peer ID(s) manually (comma-separated):`);
      if (!input) return resolve([]);
      return resolve(input.split(',').map(s => s.trim()).filter(Boolean));
    }

    // Build a simple modal
    const overlay = document.createElement('div');
    overlay.className = 'picker-overlay';
    const modal = document.createElement('div');
    modal.className = 'picker-modal';
    modal.innerHTML = `<div class="picker-title">${esc(title)}</div>`;

    const selected = new Set();
    options.sort((a, b) => (a.busy || a.away ? 1 : 0) - (b.busy || b.away ? 1 : 0));

    for (const opt of options) {
      const row = document.createElement('div');
      row.className = 'picker-row' + (opt.busy ? ' picker-busy' : '');
      row.textContent = opt.label;
      row.onclick = () => {
        if (selected.has(opt.peer_id)) {
          selected.delete(opt.peer_id);
          row.classList.remove('picker-selected');
        } else {
          selected.add(opt.peer_id);
          row.classList.add('picker-selected');
        }
      };
      modal.appendChild(row);
    }

    const btnRow = document.createElement('div');
    btnRow.className = 'picker-btns';
    const okBtn = document.createElement('button');
    okBtn.textContent = 'OK';
    okBtn.onclick = () => { overlay.remove(); resolve([...selected]); };
    const cancelBtn = document.createElement('button');
    cancelBtn.textContent = 'Cancel';
    cancelBtn.onclick = () => { overlay.remove(); resolve([]); };
    btnRow.appendChild(cancelBtn);
    btnRow.appendChild(okBtn);
    modal.appendChild(btnRow);

    overlay.appendChild(modal);
    overlay.onclick = (e) => { if (e.target === overlay) { overlay.remove(); resolve([]); } };
    document.body.appendChild(overlay);
  });
}

// ── Room list ────────────────────────────────────────────────────────────────

async function loadRooms() {
  try {
    const resp = await fetch(`${BASE}/rooms`, { headers: apiHeaders() });
    const data = await resp.json();
    renderRooms(data.rooms || []);
  } catch (e) {
    console.error('Failed to load rooms:', e);
  }
}

function renderRooms(rooms) {
  const list = document.getElementById('rooms-list');
  if (!rooms.length) {
    list.innerHTML = '<p style="color:#666; text-align:center; padding:1rem">No active rooms</p>';
    return;
  }

  list.innerHTML = rooms.map(r => {
    const capacityStr = `${r.members.length}/${r.max_members}`;

    if (r.is_invited) {
      return `
        <div class="invite-card">
          <div class="room-name">📞 ${esc(r.name || 'Voice Room')}</div>
          <div class="room-meta">${capacityStr} members — you're invited</div>
          <div class="invite-actions">
            <button onclick="joinRoom('${r.room_id}')">Join</button>
            <button onclick="declineRoom('${r.room_id}')">Decline</button>
          </div>
        </div>`;
    }

    return `
      <div class="room-card" onclick="enterRoom('${r.room_id}')">
        <div class="room-name">${esc(r.name || 'Voice Room')}</div>
        <div class="room-meta">${capacityStr} members</div>
      </div>`;
  }).join('');
}

// ── Room actions ─────────────────────────────────────────────────────────────

async function createRoom() {
  await loadPeers();
  const name = prompt('Room name (optional):') || undefined;

  // Pick peers to invite
  const invitees = await showPeerPicker('Invite peers to room');

  try {
    const resp = await fetch(`${BASE}/rooms`, {
      method: 'POST',
      headers: apiHeaders(),
      body: JSON.stringify({ name, invite: invitees, max_members: 10 }),
    });
    const room = await resp.json();
    enterRoom(room.room_id);
  } catch (e) {
    console.error('Failed to create room:', e);
  }
}

async function quickCall() {
  await loadPeers();
  const peers = await showPeerPicker('Quick call — select a peer');
  if (!peers.length) return;

  try {
    const resp = await fetch(`${BASE}/quick-call`, {
      method: 'POST',
      headers: apiHeaders(),
      body: JSON.stringify({ peer_id: peers[0] }),
    });
    if (!resp.ok) {
      const err = await resp.json();
      alert(err.error || 'Quick call failed');
      return;
    }
    const room = await resp.json();
    enterRoom(room.room_id);
  } catch (e) {
    console.error('Quick call failed:', e);
  }
}

async function joinRoom(roomId) {
  try {
    const resp = await fetch(`${BASE}/rooms/${roomId}/join`, {
      method: 'POST',
      headers: apiHeaders(),
    });
    if (!resp.ok) {
      const err = await resp.json();
      if (err.error === 'missing_tunnels') {
        alert(`Cannot join: missing tunnels to ${err.missing_peers.length} peer(s)`);
        return;
      }
      if (err.error === 'room is full') {
        alert('Room is full');
        return;
      }
      alert(err.error || 'Failed to join');
      return;
    }
    enterRoom(roomId);
  } catch (e) {
    console.error('Failed to join room:', e);
  }
}

async function enterRoom(roomId) {
  try {
    const resp = await fetch(`${BASE}/rooms/${roomId}`, { headers: apiHeaders() });
    currentRoom = await resp.json();

    document.getElementById('rooms-section').style.display = 'none';
    document.getElementById('call-section').style.display = 'block';
    renderCallView();

    await startAudio();
    connectSignaling(roomId);
    startLevelMonitor();
  } catch (e) {
    console.error('Failed to enter room:', e);
  }
}

async function leaveRoom() {
  if (!currentRoom) return;
  try {
    await fetch(`${BASE}/rooms/${currentRoom.room_id}/leave`, {
      method: 'POST',
      headers: apiHeaders(),
    });
  } catch (e) {
    console.error('Failed to leave room:', e);
  }
  cleanup();
  document.getElementById('call-section').style.display = 'none';
  document.getElementById('rooms-section').style.display = 'block';
  loadRooms();
}

async function toggleMute() {
  if (!currentRoom) return;
  isMuted = !isMuted;
  try {
    await fetch(`${BASE}/rooms/${currentRoom.room_id}/mute`, {
      method: 'POST',
      headers: apiHeaders(),
      body: JSON.stringify({ muted: isMuted }),
    });
  } catch (e) {
    console.error('Mute toggle failed:', e);
  }

  // Mute/unmute local audio track
  if (localStream) {
    localStream.getAudioTracks().forEach(t => t.enabled = !isMuted);
  }
  document.getElementById('btn-mute').textContent = isMuted ? '🔇 Unmute' : '🎤 Mute';
}

async function inviteMore() {
  if (!currentRoom) return;
  await loadPeers();

  const currentMembers = currentRoom.members.map(m => m.peer_id);
  const currentInvited = currentRoom.invited || [];
  const exclude = [...currentMembers, ...currentInvited];

  const peerIds = await showPeerPicker('Invite more peers', exclude);
  if (!peerIds.length) return;

  try {
    const resp = await fetch(`${BASE}/rooms/${currentRoom.room_id}/invite`, {
      method: 'POST',
      headers: apiHeaders(),
      body: JSON.stringify({ peer_ids: peerIds }),
    });
    if (resp.ok) {
      refreshRoom();
    } else {
      const err = await resp.json();
      alert(err.error || 'Invite failed');
    }
  } catch (e) {
    console.error('Invite failed:', e);
  }
}

function declineRoom(roomId) {
  // Just remove from UI for now — no server-side decline yet
  loadRooms();
}

// ── Call view rendering ──────────────────────────────────────────────────────

function renderCallView() {
  if (!currentRoom) return;
  document.getElementById('call-room-name').textContent = currentRoom.name || 'Voice Room';
  document.getElementById('call-member-count').textContent =
    `${currentRoom.members.length}/${currentRoom.max_members}`;

  // Disable invite button if room is full
  const inviteBtn = document.getElementById('btn-invite');
  if (inviteBtn) {
    inviteBtn.disabled = currentRoom.members.length >= currentRoom.max_members;
  }

  const list = document.getElementById('members-list');
  list.innerHTML = currentRoom.members.map(m => {
    const isYou = m.peer_id === PEER_ID;
    return `
    <div class="member" id="member-${m.peer_id}">
      <div class="member-info">
        <span class="member-name">${isYou ? '(You) ' : ''}${esc(shortId(m.peer_id))}</span>
        ${m.muted ? '<span class="member-muted">🔇</span>' : ''}
      </div>
      <div class="member-level">
        <div class="level-bar" id="level-${m.peer_id}"></div>
      </div>
    </div>`;
  }).join('');
}

// ── Audio ────────────────────────────────────────────────────────────────────

async function startAudio() {
  try {
    localStream = await navigator.mediaDevices.getUserMedia({
      audio: {
        echoCancellation: true,
        noiseSuppression: true,
        autoGainControl: true,
      }
    });

    // Set up analyser for local audio
    const audioCtx = getAudioContext();
    const source = audioCtx.createMediaStreamSource(localStream);
    const analyser = audioCtx.createAnalyser();
    analyser.fftSize = 256;
    source.connect(analyser);
    const dataArray = new Uint8Array(analyser.frequencyBinCount);
    audioAnalysers[PEER_ID] = { analyser, dataArray };
  } catch (e) {
    console.error('Failed to get audio:', e);
  }
}

let _audioCtx = null;
function getAudioContext() {
  if (!_audioCtx) _audioCtx = new (window.AudioContext || window.webkitAudioContext)();
  return _audioCtx;
}

// ── Audio level monitoring ──────────────────────────────────────────────────

function startLevelMonitor() {
  if (levelAnimFrame) cancelAnimationFrame(levelAnimFrame);

  function tick() {
    for (const [peerId, { analyser, dataArray }] of Object.entries(audioAnalysers)) {
      analyser.getByteFrequencyData(dataArray);
      let sum = 0;
      for (let i = 0; i < dataArray.length; i++) sum += dataArray[i];
      const avg = sum / dataArray.length;
      const pct = Math.min(100, (avg / 128) * 100);

      const bar = document.getElementById(`level-${peerId}`);
      if (bar) {
        bar.style.width = pct + '%';
        bar.classList.toggle('speaking', pct > 15);
      }
    }
    levelAnimFrame = requestAnimationFrame(tick);
  }
  tick();
}

function stopLevelMonitor() {
  if (levelAnimFrame) {
    cancelAnimationFrame(levelAnimFrame);
    levelAnimFrame = null;
  }
}

// ── WebRTC ───────────────────────────────────────────────────────────────────

function createPeerConnection(remotePeerId) {
  if (peerConnections[remotePeerId]) {
    peerConnections[remotePeerId].close();
    delete peerConnections[remotePeerId];
  }

  const pc = new RTCPeerConnection({ iceServers: [] });

  if (localStream) {
    localStream.getTracks().forEach(track => pc.addTrack(track, localStream));
  }

  pc.ontrack = (event) => {
    const oldAudio = document.getElementById(`audio-${remotePeerId}`);
    if (oldAudio) oldAudio.remove();

    const audio = document.createElement('audio');
    audio.srcObject = event.streams[0];
    audio.autoplay = true;
    audio.id = `audio-${remotePeerId}`;
    document.body.appendChild(audio);

    try {
      const audioCtx = getAudioContext();
      const source = audioCtx.createMediaStreamSource(event.streams[0]);
      const analyser = audioCtx.createAnalyser();
      analyser.fftSize = 256;
      source.connect(analyser);
      const dataArray = new Uint8Array(analyser.frequencyBinCount);
      audioAnalysers[remotePeerId] = { analyser, dataArray };
    } catch (e) {
      console.warn('Could not create analyser for remote peer:', e);
    }
  };

  pc.onicecandidate = (event) => {
    if (event.candidate && ws) {
      ws.send(JSON.stringify({
        type: 'ice-candidate',
        to: remotePeerId,
        candidate: JSON.stringify(event.candidate),
      }));
    }
  };

  pc.onconnectionstatechange = () => {
    if (pc.connectionState === 'failed' || pc.connectionState === 'disconnected') {
      console.warn(`Connection to ${remotePeerId}: ${pc.connectionState}`);
    }
  };

  peerConnections[remotePeerId] = pc;
  return pc;
}

async function handleOffer(from, sdp) {
  const pc = createPeerConnection(from);
  await pc.setRemoteDescription(new RTCSessionDescription({ type: 'offer', sdp }));
  const answer = await pc.createAnswer();
  await pc.setLocalDescription(answer);
  if (ws) {
    ws.send(JSON.stringify({ type: 'sdp-answer', to: from, sdp: answer.sdp }));
  }
}

async function handleAnswer(from, sdp) {
  const pc = peerConnections[from];
  if (pc) {
    await pc.setRemoteDescription(new RTCSessionDescription({ type: 'answer', sdp }));
  }
}

async function handleIceCandidate(from, candidateStr) {
  const pc = peerConnections[from];
  if (pc) {
    const candidate = JSON.parse(candidateStr);
    await pc.addIceCandidate(new RTCIceCandidate(candidate));
  }
}

// ── Audio cues ───────────────────────────────────────────────────────────────

function playTone(freq, durationMs) {
  try {
    const ctx = getAudioContext();
    const osc = ctx.createOscillator();
    const gain = ctx.createGain();
    osc.type = 'sine';
    osc.frequency.value = freq;
    gain.gain.value = 0.08;
    gain.gain.exponentialRampToValueAtTime(0.001, ctx.currentTime + durationMs / 1000);
    osc.connect(gain);
    gain.connect(ctx.destination);
    osc.start();
    osc.stop(ctx.currentTime + durationMs / 1000);
  } catch (_) { /* audio context may not be available */ }
}

function playJoinCue() { playTone(880, 150); setTimeout(() => playTone(1100, 120), 160); }
function playLeaveCue() { playTone(600, 150); setTimeout(() => playTone(440, 180), 160); }

// ── Signaling WebSocket ──────────────────────────────────────────────────────

function connectSignaling(roomId) {
  // WebSocket connects directly to the voice cap's port because the daemon
  // HTTP proxy doesn't support WebSocket upgrade. The cap port (7005) is
  // read from the manifest at install time; for now hardcoded as the default.
  // TODO: replace with a /ws-info endpoint or daemon-level WS proxy support.
  const wsPort = 7005;
  const protocol = location.protocol === 'https:' ? 'wss:' : 'ws:';
  const url = `${protocol}//localhost:${wsPort}/rooms/${roomId}/signal`;
  ws = new WebSocket(url);

  ws.onopen = () => {
    ws.send(JSON.stringify({ peer_id: PEER_ID }));
  };

  ws.onmessage = (event) => {
    const msg = JSON.parse(event.data);

    switch (msg.type) {
      case 'peer-joined':
        if (msg.peer_id !== PEER_ID) {
          sendOffer(msg.peer_id);
          refreshRoom();
          playJoinCue();
        }
        break;

      case 'peer-left':
        removePeer(msg.peer_id);
        refreshRoom();
        playLeaveCue();
        break;

      case 'sdp-offer':
        handleOffer(msg.from, msg.sdp);
        break;

      case 'sdp-answer':
        handleAnswer(msg.from, msg.sdp);
        break;

      case 'ice-candidate':
        handleIceCandidate(msg.from, msg.candidate);
        break;

      case 'mute-changed':
        refreshRoom();
        break;

      case 'room-closed':
        cleanup();
        document.getElementById('call-section').style.display = 'none';
        document.getElementById('rooms-section').style.display = 'block';
        loadRooms();
        break;

      case 'error':
        console.error('Signal error:', msg.message);
        break;
    }
  };

  ws.onclose = () => {
    console.log('Signaling WebSocket closed');
  };
}

async function sendOffer(remotePeerId) {
  const pc = createPeerConnection(remotePeerId);
  const offer = await pc.createOffer();
  await pc.setLocalDescription(offer);
  if (ws) {
    ws.send(JSON.stringify({ type: 'sdp-offer', to: remotePeerId, sdp: offer.sdp }));
  }
}

function removePeer(peerId) {
  const pc = peerConnections[peerId];
  if (pc) {
    pc.close();
    delete peerConnections[peerId];
  }
  delete audioAnalysers[peerId];
  const audio = document.getElementById(`audio-${peerId}`);
  if (audio) audio.remove();
}

async function refreshRoom() {
  if (!currentRoom) return;
  try {
    const resp = await fetch(`${BASE}/rooms/${currentRoom.room_id}`, { headers: apiHeaders() });
    currentRoom = await resp.json();
    renderCallView();
  } catch (e) {
    console.error('Failed to refresh room:', e);
  }
}

function cleanup() {
  stopLevelMonitor();

  Object.keys(peerConnections).forEach(pid => removePeer(pid));
  peerConnections = {};
  audioAnalysers = {};

  if (ws) {
    ws.close();
    ws = null;
  }

  if (localStream) {
    localStream.getTracks().forEach(t => t.stop());
    localStream = null;
  }

  currentRoom = null;
  isMuted = false;
  document.getElementById('btn-mute').textContent = '🎤 Mute';
}

// ── Init ─────────────────────────────────────────────────────────────────────

fetchMyId().then(() => {
  loadRooms();
  loadPeers();
});
// Poll room list every 10s when not in a call, presence every 30s
setInterval(() => {
  if (!currentRoom) loadRooms();
}, 10000);
setInterval(loadPeers, 30000);
