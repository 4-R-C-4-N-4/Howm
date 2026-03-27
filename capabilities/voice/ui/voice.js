// Voice capability UI — room management + WebRTC audio
'use strict';

const API = '';  // relative to capability base path
let currentRoom = null;
let isMuted = false;
let ws = null;
let peerConnections = {};  // peer_id -> RTCPeerConnection
let localStream = null;

// ── Peer ID ──────────────────────────────────────────────────────────────────

// In production, the daemon proxy injects the peer identity.
// For now, read from localStorage or prompt.
function getPeerId() {
  let id = localStorage.getItem('howm_peer_id');
  if (!id) {
    id = 'peer-' + Math.random().toString(36).slice(2, 10);
    localStorage.setItem('howm_peer_id', id);
  }
  return id;
}

const PEER_ID = getPeerId();

function apiHeaders() {
  return { 'Content-Type': 'application/json', 'X-Peer-Id': PEER_ID };
}

// ── Room list ────────────────────────────────────────────────────────────────

async function loadRooms() {
  try {
    const resp = await fetch(`${API}/rooms`, { headers: apiHeaders() });
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
    const isMember = r.members.some(m => m.peer_id === PEER_ID);
    const isInvited = r.invited.includes(PEER_ID);

    if (isInvited) {
      return `
        <div class="invite-card">
          <div class="room-name">📞 ${r.name || 'Voice Room'}</div>
          <div class="room-members">${r.members.length} member(s) — you're invited</div>
          <div class="invite-actions">
            <button onclick="joinRoom('${r.room_id}')">Join</button>
            <button onclick="declineRoom('${r.room_id}')">Decline</button>
          </div>
        </div>`;
    }

    return `
      <div class="room-card" onclick="enterRoom('${r.room_id}')">
        <div class="room-name">${r.name || 'Voice Room'}</div>
        <div class="room-members">${r.members.length} member(s)</div>
      </div>`;
  }).join('');
}

// ── Room actions ─────────────────────────────────────────────────────────────

async function createRoom() {
  const name = prompt('Room name (optional):') || undefined;
  try {
    const resp = await fetch(`${API}/rooms`, {
      method: 'POST',
      headers: apiHeaders(),
      body: JSON.stringify({ name, invite: [], max_members: 10 }),
    });
    const room = await resp.json();
    enterRoom(room.room_id);
  } catch (e) {
    console.error('Failed to create room:', e);
  }
}

async function joinRoom(roomId) {
  try {
    await fetch(`${API}/rooms/${roomId}/join`, {
      method: 'POST',
      headers: apiHeaders(),
    });
    enterRoom(roomId);
  } catch (e) {
    console.error('Failed to join room:', e);
  }
}

async function enterRoom(roomId) {
  try {
    const resp = await fetch(`${API}/rooms/${roomId}`, { headers: apiHeaders() });
    currentRoom = await resp.json();

    document.getElementById('rooms-section').style.display = 'none';
    document.getElementById('call-section').style.display = 'block';
    renderCallView();

    await startAudio();
    connectSignaling(roomId);
  } catch (e) {
    console.error('Failed to enter room:', e);
  }
}

async function leaveRoom() {
  if (!currentRoom) return;
  try {
    await fetch(`${API}/rooms/${currentRoom.room_id}/leave`, {
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
    await fetch(`${API}/rooms/${currentRoom.room_id}/mute`, {
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

function declineRoom(roomId) {
  // Just remove from UI for now — no server-side decline yet
  loadRooms();
}

// ── Call view rendering ──────────────────────────────────────────────────────

function renderCallView() {
  if (!currentRoom) return;
  document.getElementById('call-room-name').textContent = currentRoom.name || 'Voice Room';
  document.getElementById('call-member-count').textContent = `${currentRoom.members.length} member(s)`;

  const list = document.getElementById('members-list');
  list.innerHTML = currentRoom.members.map(m => `
    <div class="member">
      <span class="member-name">${m.peer_id === PEER_ID ? '(You) ' : ''}${m.peer_id}</span>
      ${m.muted ? '<span class="member-muted">muted</span>' : ''}
    </div>
  `).join('');
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
  } catch (e) {
    console.error('Failed to get audio:', e);
  }
}

// ── WebRTC ───────────────────────────────────────────────────────────────────

function createPeerConnection(remotePeerId) {
  const pc = new RTCPeerConnection({ iceServers: [] });

  // Add local audio track
  if (localStream) {
    localStream.getTracks().forEach(track => pc.addTrack(track, localStream));
  }

  // Handle incoming audio
  pc.ontrack = (event) => {
    const audio = document.createElement('audio');
    audio.srcObject = event.streams[0];
    audio.autoplay = true;
    audio.id = `audio-${remotePeerId}`;
    document.body.appendChild(audio);
  };

  // ICE candidates
  pc.onicecandidate = (event) => {
    if (event.candidate && ws) {
      ws.send(JSON.stringify({
        type: 'ice-candidate',
        to: remotePeerId,
        candidate: JSON.stringify(event.candidate),
      }));
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

// ── Signaling WebSocket ──────────────────────────────────────────────────────

function connectSignaling(roomId) {
  const protocol = location.protocol === 'https:' ? 'wss:' : 'ws:';
  const url = `${protocol}//${location.host}/rooms/${roomId}/signal`;
  ws = new WebSocket(url);

  ws.onopen = () => {
    // Identify ourselves
    ws.send(JSON.stringify({ peer_id: PEER_ID }));
  };

  ws.onmessage = (event) => {
    const msg = JSON.parse(event.data);

    switch (msg.type) {
      case 'peer-joined':
        // New peer — we (existing member) send them an offer
        if (msg.peer_id !== PEER_ID) {
          sendOffer(msg.peer_id);
          // Refresh room state
          refreshRoom();
        }
        break;

      case 'peer-left':
        removePeer(msg.peer_id);
        refreshRoom();
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
  const audio = document.getElementById(`audio-${peerId}`);
  if (audio) audio.remove();
}

async function refreshRoom() {
  if (!currentRoom) return;
  try {
    const resp = await fetch(`${API}/rooms/${currentRoom.room_id}`, { headers: apiHeaders() });
    currentRoom = await resp.json();
    renderCallView();
  } catch (e) {
    console.error('Failed to refresh room:', e);
  }
}

function cleanup() {
  // Close all peer connections
  Object.keys(peerConnections).forEach(pid => removePeer(pid));
  peerConnections = {};

  // Close WebSocket
  if (ws) {
    ws.close();
    ws = null;
  }

  // Stop local audio
  if (localStream) {
    localStream.getTracks().forEach(t => t.stop());
    localStream = null;
  }

  currentRoom = null;
  isMuted = false;
  document.getElementById('btn-mute').textContent = '🎤 Mute';
}

// ── Init ─────────────────────────────────────────────────────────────────────

loadRooms();
// Poll room list every 10 seconds when not in a call
setInterval(() => {
  if (!currentRoom) loadRooms();
}, 10000);
