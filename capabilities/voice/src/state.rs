//! Voice room state — in-memory room management.

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

// ── Configuration ────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct VoiceConfig {
    pub max_room_size: u16,
    pub room_timeout_secs: u64,
    pub invite_timeout_secs: u64,
}

impl Default for VoiceConfig {
    fn default() -> Self {
        Self {
            max_room_size: 10,
            room_timeout_secs: 3600,
            invite_timeout_secs: 300,
        }
    }
}

impl VoiceConfig {
    pub fn from_env() -> Self {
        Self {
            max_room_size: std::env::var("VOICE_MAX_ROOM_SIZE")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(10),
            room_timeout_secs: std::env::var("VOICE_ROOM_TIMEOUT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(3600),
            invite_timeout_secs: std::env::var("VOICE_INVITE_TIMEOUT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(300),
        }
    }
}

// ── Data model ───────────────────────────────────────────────────────────────

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomMember {
    pub peer_id: String,
    pub joined_at: u64,
    pub muted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Room {
    pub room_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub created_by: String,
    pub created_at: u64,
    pub members: Vec<RoomMember>,
    pub invited: Vec<String>,
    pub max_members: u16,
}

// ── Room store ───────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct RoomStore {
    rooms: Arc<RwLock<HashMap<String, Room>>>,
    config: VoiceConfig,
}

impl RoomStore {
    pub fn new(config: VoiceConfig) -> Self {
        Self {
            rooms: Arc::new(RwLock::new(HashMap::new())),
            config,
        }
    }

    pub fn config(&self) -> &VoiceConfig {
        &self.config
    }

    /// Create a new room. The creator is auto-joined.
    pub fn create_room(
        &self,
        creator_peer_id: &str,
        name: Option<String>,
        invited: Vec<String>,
        max_members: Option<u16>,
    ) -> Room {
        let room_id = uuid::Uuid::now_v7().to_string();
        let now = now_secs();
        let max = max_members.unwrap_or(self.config.max_room_size);

        let room = Room {
            room_id: room_id.clone(),
            name,
            created_by: creator_peer_id.to_string(),
            created_at: now,
            members: vec![RoomMember {
                peer_id: creator_peer_id.to_string(),
                joined_at: now,
                muted: false,
            }],
            invited,
            max_members: max,
        };

        self.rooms.write().insert(room_id, room.clone());
        room
    }

    /// Get a room by ID.
    pub fn get_room(&self, room_id: &str) -> Option<Room> {
        self.rooms.read().get(room_id).cloned()
    }

    /// List rooms where the given peer is a member or is invited.
    pub fn list_rooms_for_peer(&self, peer_id: &str) -> Vec<Room> {
        self.rooms
            .read()
            .values()
            .filter(|r| {
                r.members.iter().any(|m| m.peer_id == peer_id)
                    || r.invited.iter().any(|i| i == peer_id)
            })
            .cloned()
            .collect()
    }

    /// Join a room. Returns updated room or error message.
    pub fn join_room(&self, room_id: &str, peer_id: &str) -> Result<Room, String> {
        let mut rooms = self.rooms.write();
        let room = rooms
            .get_mut(room_id)
            .ok_or_else(|| "room not found".to_string())?;

        // Must be invited
        if !room.invited.iter().any(|i| i == peer_id) {
            return Err("not invited to this room".to_string());
        }

        // Check room capacity
        if room.members.len() >= room.max_members as usize {
            return Err("room is full".to_string());
        }

        // Already a member?
        if room.members.iter().any(|m| m.peer_id == peer_id) {
            return Ok(room.clone());
        }

        // Remove from invited, add to members
        room.invited.retain(|i| i != peer_id);
        room.members.push(RoomMember {
            peer_id: peer_id.to_string(),
            joined_at: now_secs(),
            muted: false,
        });

        Ok(room.clone())
    }

    /// Leave a room. Returns true if the room was destroyed (last member left).
    pub fn leave_room(&self, room_id: &str, peer_id: &str) -> Result<bool, String> {
        let mut rooms = self.rooms.write();
        let room = rooms
            .get_mut(room_id)
            .ok_or_else(|| "room not found".to_string())?;

        room.members.retain(|m| m.peer_id != peer_id);

        if room.members.is_empty() {
            rooms.remove(room_id);
            Ok(true) // room destroyed
        } else {
            Ok(false)
        }
    }

    /// Close a room (creator only). Returns error if not the creator.
    pub fn close_room(&self, room_id: &str, peer_id: &str) -> Result<Room, String> {
        let mut rooms = self.rooms.write();
        let room = rooms
            .get(room_id)
            .ok_or_else(|| "room not found".to_string())?;

        if room.created_by != peer_id {
            return Err("only the room creator can close the room".to_string());
        }

        let room = rooms.remove(room_id).unwrap();
        Ok(room)
    }

    /// Invite additional peers to a room.
    pub fn invite_peers(&self, room_id: &str, peer_ids: Vec<String>) -> Result<Room, String> {
        let mut rooms = self.rooms.write();
        let room = rooms
            .get_mut(room_id)
            .ok_or_else(|| "room not found".to_string())?;

        for pid in peer_ids {
            if !room.invited.contains(&pid) && !room.members.iter().any(|m| m.peer_id == pid) {
                room.invited.push(pid);
            }
        }

        Ok(room.clone())
    }

    /// Toggle mute for a member.
    pub fn set_mute(&self, room_id: &str, peer_id: &str, muted: bool) -> Result<Room, String> {
        let mut rooms = self.rooms.write();
        let room = rooms
            .get_mut(room_id)
            .ok_or_else(|| "room not found".to_string())?;

        let member = room
            .members
            .iter_mut()
            .find(|m| m.peer_id == peer_id)
            .ok_or_else(|| "not a member of this room".to_string())?;

        member.muted = muted;
        Ok(room.clone())
    }

    /// Remove empty rooms older than the timeout.
    pub fn cleanup_stale_rooms(&self) -> usize {
        let now = now_secs();
        let timeout = self.config.room_timeout_secs;
        let mut rooms = self.rooms.write();
        let before = rooms.len();
        rooms.retain(|_, r| {
            // Keep rooms that have members or are younger than the timeout
            !r.members.is_empty() || (now - r.created_at) < timeout
        });
        before - rooms.len()
    }

    /// Remove expired invitations from all rooms.
    /// TODO: per-invite timestamps for proper expiry tracking.
    pub fn cleanup_expired_invites(&self) {
        let _now = now_secs();
        let _timeout = self.config.invite_timeout_secs;
        // For now, invites expire when the room itself is cleaned up.
        // Per-invite timestamps will be added when inter-node invites land.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_store() -> RoomStore {
        RoomStore::new(VoiceConfig::default())
    }

    #[test]
    fn test_create_room() {
        let store = test_store();
        let room = store.create_room("alice", Some("Test".into()), vec!["bob".into()], None);
        assert_eq!(room.created_by, "alice");
        assert_eq!(room.members.len(), 1);
        assert_eq!(room.members[0].peer_id, "alice");
        assert_eq!(room.invited, vec!["bob"]);
        assert_eq!(room.max_members, 10);
    }

    #[test]
    fn test_join_room() {
        let store = test_store();
        let room = store.create_room("alice", None, vec!["bob".into()], None);
        let updated = store.join_room(&room.room_id, "bob").unwrap();
        assert_eq!(updated.members.len(), 2);
        assert!(updated.invited.is_empty());
    }

    #[test]
    fn test_join_not_invited() {
        let store = test_store();
        let room = store.create_room("alice", None, vec![], None);
        let err = store.join_room(&room.room_id, "charlie").unwrap_err();
        assert!(err.contains("not invited"));
    }

    #[test]
    fn test_leave_room_destroys_when_empty() {
        let store = test_store();
        let room = store.create_room("alice", None, vec![], None);
        let destroyed = store.leave_room(&room.room_id, "alice").unwrap();
        assert!(destroyed);
        assert!(store.get_room(&room.room_id).is_none());
    }

    #[test]
    fn test_leave_room_keeps_room_with_others() {
        let store = test_store();
        let room = store.create_room("alice", None, vec!["bob".into()], None);
        store.join_room(&room.room_id, "bob").unwrap();
        let destroyed = store.leave_room(&room.room_id, "alice").unwrap();
        assert!(!destroyed);
        let r = store.get_room(&room.room_id).unwrap();
        assert_eq!(r.members.len(), 1);
        assert_eq!(r.members[0].peer_id, "bob");
    }

    #[test]
    fn test_close_room_creator_only() {
        let store = test_store();
        let room = store.create_room("alice", None, vec!["bob".into()], None);
        store.join_room(&room.room_id, "bob").unwrap();

        // Bob can't close
        let err = store.close_room(&room.room_id, "bob").unwrap_err();
        assert!(err.contains("creator"));

        // Alice can
        let closed = store.close_room(&room.room_id, "alice").unwrap();
        assert_eq!(closed.members.len(), 2);
        assert!(store.get_room(&room.room_id).is_none());
    }

    #[test]
    fn test_mute_toggle() {
        let store = test_store();
        let room = store.create_room("alice", None, vec![], None);
        let updated = store.set_mute(&room.room_id, "alice", true).unwrap();
        assert!(updated.members[0].muted);
        let updated = store.set_mute(&room.room_id, "alice", false).unwrap();
        assert!(!updated.members[0].muted);
    }

    #[test]
    fn test_invite_additional_peers() {
        let store = test_store();
        let room = store.create_room("alice", None, vec![], None);
        let updated = store
            .invite_peers(&room.room_id, vec!["bob".into(), "carol".into()])
            .unwrap();
        assert_eq!(updated.invited.len(), 2);
    }

    #[test]
    fn test_list_rooms_for_peer() {
        let store = test_store();
        store.create_room("alice", Some("Room1".into()), vec!["bob".into()], None);
        store.create_room("carol", Some("Room2".into()), vec![], None);

        let alice_rooms = store.list_rooms_for_peer("alice");
        assert_eq!(alice_rooms.len(), 1);

        let bob_rooms = store.list_rooms_for_peer("bob");
        assert_eq!(bob_rooms.len(), 1); // invited

        let carol_rooms = store.list_rooms_for_peer("carol");
        assert_eq!(carol_rooms.len(), 1);
    }

    #[test]
    fn test_room_full_rejects_join() {
        let store = RoomStore::new(VoiceConfig {
            max_room_size: 2,
            ..VoiceConfig::default()
        });
        let room = store.create_room("alice", None, vec!["bob".into(), "carol".into()], Some(2));
        store.join_room(&room.room_id, "bob").unwrap();
        let err = store.join_room(&room.room_id, "carol").unwrap_err();
        assert!(err.contains("full"));
    }
}
