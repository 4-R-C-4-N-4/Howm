//! howm-access — Peer group-based access control for Howm.
//!
//! Provides the shared `AccessDb` that both the daemon trust gate and
//! capability handlers use to evaluate per-peer permissions.
//!
//! See `docs/access/BRD-access-control.md` for the full specification.

pub mod db;
pub mod schema;
pub mod types;

pub use db::AccessDb;
pub use types::*;

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_db() -> (AccessDb, TempDir) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("access.db");
        let db = AccessDb::open(&path).unwrap();
        (db, dir)
    }

    fn fake_peer(n: u8) -> Vec<u8> {
        let mut id = vec![0u8; 32];
        id[31] = n;
        id
    }

    // ── Built-in groups ──────────────────────────────────────────────────

    #[test]
    fn built_in_groups_created_on_init() {
        let (db, _dir) = test_db();
        let groups = db.list_groups().unwrap();

        assert_eq!(groups.len(), 3);

        let names: Vec<&str> = groups.iter().map(|g| g.name.as_str()).collect();
        assert!(names.contains(&"howm.default"));
        assert!(names.contains(&"howm.friends"));
        assert!(names.contains(&"howm.trusted"));

        for g in &groups {
            assert!(g.built_in);
        }
    }

    #[test]
    fn built_in_groups_not_deletable() {
        let (db, _dir) = test_db();
        let result = db.delete_group(&GROUP_DEFAULT);
        assert!(result.is_err());

        let result = db.delete_group(&GROUP_FRIENDS);
        assert!(result.is_err());

        let result = db.delete_group(&GROUP_TRUSTED);
        assert!(result.is_err());
    }

    #[test]
    fn built_in_groups_rules_not_modifiable() {
        let (db, _dir) = test_db();
        let rules = vec![CapabilityRule {
            capability_name: "test.cap.1".to_string(),
            allow: true,
            rate_limit: None,
            ttl: None,
        }];
        let result = db.update_group(&GROUP_DEFAULT, None, None, Some(&rules));
        assert!(result.is_err());
    }

    #[test]
    fn built_in_groups_name_description_editable() {
        let (db, _dir) = test_db();
        let result = db
            .update_group(
                &GROUP_DEFAULT,
                Some("my-default"),
                Some(Some("custom desc")),
                None,
            )
            .unwrap();
        let g = result.unwrap();
        assert_eq!(g.name, "my-default");
        assert_eq!(g.description.as_deref(), Some("custom desc"));
    }

    // ── Default fallback (BRD §4.4) ─────────────────────────────────────

    #[test]
    fn peer_with_no_membership_gets_default_caps() {
        let (db, _dir) = test_db();
        let peer = fake_peer(1);

        // Should be allowed (in howm.default rules)
        assert!(db
            .resolve_permission(&peer, "core.session.heartbeat.1")
            .is_allowed());
        assert!(db
            .resolve_permission(&peer, "core.session.attest.1")
            .is_allowed());
        assert!(db
            .resolve_permission(&peer, "core.session.latency.1")
            .is_allowed());
        assert!(db
            .resolve_permission(&peer, "core.network.endpoint.1")
            .is_allowed());
        assert!(db
            .resolve_permission(&peer, "core.session.timesync.1")
            .is_allowed());

        // Should be denied (not in howm.default rules)
        assert!(!db
            .resolve_permission(&peer, "howm.social.feed.1")
            .is_allowed());
        assert!(!db
            .resolve_permission(&peer, "howm.social.messaging.1")
            .is_allowed());
        assert!(!db
            .resolve_permission(&peer, "howm.social.files.1")
            .is_allowed());
        assert!(!db
            .resolve_permission(&peer, "howm.world.room.1")
            .is_allowed());
        assert!(!db
            .resolve_permission(&peer, "core.network.peerexchange.1")
            .is_allowed());
        assert!(!db
            .resolve_permission(&peer, "core.network.relay.1")
            .is_allowed());
    }

    #[test]
    fn unknown_capability_denied() {
        let (db, _dir) = test_db();
        let peer = fake_peer(1);
        assert!(!db
            .resolve_permission(&peer, "totally.made.up.1")
            .is_allowed());
    }

    // ── Friends tier ─────────────────────────────────────────────────────

    #[test]
    fn friends_peer_gets_social_caps() {
        let (db, _dir) = test_db();
        let peer = fake_peer(2);
        db.assign_peer_to_group(&peer, &GROUP_FRIENDS).unwrap();

        // Default caps still work
        assert!(db
            .resolve_permission(&peer, "core.session.heartbeat.1")
            .is_allowed());

        // Friends caps now work
        assert!(db
            .resolve_permission(&peer, "howm.social.feed.1")
            .is_allowed());
        assert!(db
            .resolve_permission(&peer, "howm.social.messaging.1")
            .is_allowed());
        assert!(db
            .resolve_permission(&peer, "howm.social.files.1")
            .is_allowed());
        assert!(db
            .resolve_permission(&peer, "howm.world.room.1")
            .is_allowed());
        assert!(db
            .resolve_permission(&peer, "core.network.peerexchange.1")
            .is_allowed());

        // Relay still denied
        assert!(!db
            .resolve_permission(&peer, "core.network.relay.1")
            .is_allowed());
    }

    // ── Trusted tier ─────────────────────────────────────────────────────

    #[test]
    fn trusted_peer_gets_relay() {
        let (db, _dir) = test_db();
        let peer = fake_peer(3);
        db.assign_peer_to_group(&peer, &GROUP_TRUSTED).unwrap();

        // Relay now allowed
        assert!(db
            .resolve_permission(&peer, "core.network.relay.1")
            .is_allowed());

        // Default caps still work
        assert!(db
            .resolve_permission(&peer, "core.session.heartbeat.1")
            .is_allowed());

        // Friends caps NOT allowed (trusted only adds relay, doesn't include friends caps)
        // Peer needs to be in BOTH trusted and friends for social caps
        assert!(!db
            .resolve_permission(&peer, "howm.social.feed.1")
            .is_allowed());
    }

    #[test]
    fn trusted_plus_friends_gets_everything() {
        let (db, _dir) = test_db();
        let peer = fake_peer(4);
        db.assign_peer_to_group(&peer, &GROUP_FRIENDS).unwrap();
        db.assign_peer_to_group(&peer, &GROUP_TRUSTED).unwrap();

        assert!(db
            .resolve_permission(&peer, "core.session.heartbeat.1")
            .is_allowed());
        assert!(db
            .resolve_permission(&peer, "howm.social.feed.1")
            .is_allowed());
        assert!(db
            .resolve_permission(&peer, "core.network.relay.1")
            .is_allowed());
    }

    // ── Most permissive wins (BRD §4.4) ──────────────────────────────────

    #[test]
    fn most_permissive_wins_custom_group_grants_access() {
        let (db, _dir) = test_db();
        let peer = fake_peer(5);

        // Peer is in howm.default (implicit) which denies social.
        // Create a custom group that grants files only.
        let custom = db
            .create_group(
                "files-only",
                Some("just files"),
                &[CapabilityRule {
                    capability_name: "howm.social.files.1".to_string(),
                    allow: true,
                    rate_limit: None,
                    ttl: None,
                }],
            )
            .unwrap();

        db.assign_peer_to_group(&peer, &custom.group_id).unwrap();

        // Files allowed via custom group, even though default denies
        assert!(db
            .resolve_permission(&peer, "howm.social.files.1")
            .is_allowed());

        // Other social caps still denied
        assert!(!db
            .resolve_permission(&peer, "howm.social.feed.1")
            .is_allowed());

        // Default caps still allowed
        assert!(db
            .resolve_permission(&peer, "core.session.heartbeat.1")
            .is_allowed());
    }

    // ── Custom group CRUD ────────────────────────────────────────────────

    #[test]
    fn custom_group_create_and_delete() {
        let (db, _dir) = test_db();
        let group = db
            .create_group(
                "testers",
                Some("QA team"),
                &[CapabilityRule {
                    capability_name: "howm.social.feed.1".to_string(),
                    allow: true,
                    rate_limit: None,
                    ttl: None,
                }],
            )
            .unwrap();

        assert!(!group.built_in);
        assert_eq!(group.name, "testers");
        assert_eq!(group.capabilities.len(), 1);

        // List should have 4 groups now
        let all = db.list_groups().unwrap();
        assert_eq!(all.len(), 4);

        // Delete
        assert!(db.delete_group(&group.group_id).unwrap());
        let all = db.list_groups().unwrap();
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn custom_group_update_rules() {
        let (db, _dir) = test_db();
        let group = db.create_group("mutable", None, &[]).unwrap();

        assert_eq!(group.capabilities.len(), 0);

        // Add rules
        let new_rules = vec![
            CapabilityRule {
                capability_name: "howm.social.feed.1".to_string(),
                allow: true,
                rate_limit: None,
                ttl: None,
            },
            CapabilityRule {
                capability_name: "howm.social.messaging.1".to_string(),
                allow: true,
                rate_limit: None,
                ttl: None,
            },
        ];

        let updated = db
            .update_group(&group.group_id, None, None, Some(&new_rules))
            .unwrap()
            .unwrap();
        assert_eq!(updated.capabilities.len(), 2);
    }

    // ── Membership ───────────────────────────────────────────────────────

    #[test]
    fn assign_and_remove_peer_from_group() {
        let (db, _dir) = test_db();
        let peer = fake_peer(10);

        db.assign_peer_to_group(&peer, &GROUP_FRIENDS).unwrap();
        let groups = db.list_peer_groups(&peer).unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].group_id, GROUP_FRIENDS);

        // Duplicate assign is no-op
        db.assign_peer_to_group(&peer, &GROUP_FRIENDS).unwrap();
        let groups = db.list_peer_groups(&peer).unwrap();
        assert_eq!(groups.len(), 1);

        // Remove
        assert!(db.remove_peer_from_group(&peer, &GROUP_FRIENDS).unwrap());
        let groups = db.list_peer_groups(&peer).unwrap();
        assert_eq!(groups.len(), 0);

        // Remove again — returns false
        assert!(!db.remove_peer_from_group(&peer, &GROUP_FRIENDS).unwrap());
    }

    #[test]
    fn remove_peer_from_all_groups() {
        let (db, _dir) = test_db();
        let peer = fake_peer(11);

        db.assign_peer_to_group(&peer, &GROUP_FRIENDS).unwrap();
        db.assign_peer_to_group(&peer, &GROUP_TRUSTED).unwrap();

        let deleted = db.remove_peer_from_all_groups(&peer).unwrap();
        assert_eq!(deleted, 2);

        let groups = db.list_peer_groups(&peer).unwrap();
        assert_eq!(groups.len(), 0);
    }

    #[test]
    fn peer_has_memberships_check() {
        let (db, _dir) = test_db();
        let peer = fake_peer(12);

        assert!(!db.peer_has_memberships(&peer).unwrap());

        db.assign_peer_to_group(&peer, &GROUP_DEFAULT).unwrap();
        assert!(db.peer_has_memberships(&peer).unwrap());
    }

    // ── Permission changes take effect immediately ──────────────────────

    #[test]
    fn permission_changes_immediate_no_cache() {
        let (db, _dir) = test_db();
        let peer = fake_peer(20);

        // Initially denied social
        assert!(!db
            .resolve_permission(&peer, "howm.social.feed.1")
            .is_allowed());

        // Promote to friends
        db.assign_peer_to_group(&peer, &GROUP_FRIENDS).unwrap();
        assert!(db
            .resolve_permission(&peer, "howm.social.feed.1")
            .is_allowed());

        // Demote back
        db.remove_peer_from_group(&peer, &GROUP_FRIENDS).unwrap();
        assert!(!db
            .resolve_permission(&peer, "howm.social.feed.1")
            .is_allowed());
    }

    // ── Effective permissions ────────────────────────────────────────────

    #[test]
    fn effective_permissions_for_default_peer() {
        let (db, _dir) = test_db();
        let peer = fake_peer(30);

        let perms = db.get_peer_effective_permissions(&peer).unwrap();

        // Should have entries for all capabilities defined in rules
        assert!(perms["core.session.heartbeat.1"].is_allowed());
        assert!(perms["core.session.attest.1"].is_allowed());
        assert!(!perms["howm.social.feed.1"].is_allowed());
        assert!(!perms["core.network.relay.1"].is_allowed());
    }

    #[test]
    fn effective_permissions_for_friends_peer() {
        let (db, _dir) = test_db();
        let peer = fake_peer(31);
        db.assign_peer_to_group(&peer, &GROUP_FRIENDS).unwrap();

        let perms = db.get_peer_effective_permissions(&peer).unwrap();

        assert!(perms["core.session.heartbeat.1"].is_allowed());
        assert!(perms["howm.social.feed.1"].is_allowed());
        assert!(perms["howm.social.messaging.1"].is_allowed());
        assert!(!perms["core.network.relay.1"].is_allowed());
    }

    // ── Resolve all permissions (batch) ──────────────────────────────────

    #[test]
    fn resolve_all_permissions_batch() {
        let (db, _dir) = test_db();
        let peer = fake_peer(40);

        let caps = &[
            "core.session.heartbeat.1",
            "howm.social.feed.1",
            "core.network.relay.1",
        ];
        let perms = db.resolve_all_permissions(&peer, caps);

        assert_eq!(perms.len(), 3);
        assert!(perms["core.session.heartbeat.1"].is_allowed());
        assert!(!perms["howm.social.feed.1"].is_allowed());
        assert!(!perms["core.network.relay.1"].is_allowed());
    }

    // ── Cascade delete: removing group removes memberships ───────────────

    #[test]
    fn deleting_group_cascades_memberships() {
        let (db, _dir) = test_db();
        let peer = fake_peer(50);

        let group = db.create_group("ephemeral", None, &[]).unwrap();
        db.assign_peer_to_group(&peer, &group.group_id).unwrap();

        assert!(db.peer_has_memberships(&peer).unwrap());

        db.delete_group(&group.group_id).unwrap();

        // Membership should be gone
        assert!(!db.peer_has_memberships(&peer).unwrap());
    }

    // ── Nonexistent group ────────────────────────────────────────────────

    #[test]
    fn get_nonexistent_group_returns_none() {
        let (db, _dir) = test_db();
        let fake_id = uuid::Uuid::new_v4();
        assert!(db.get_group(&fake_id).unwrap().is_none());
    }

    #[test]
    fn delete_nonexistent_group_returns_false() {
        let (db, _dir) = test_db();
        let fake_id = uuid::Uuid::new_v4();
        assert!(!db.delete_group(&fake_id).unwrap());
    }

    // ── Re-open persistence ──────────────────────────────────────────────

    #[test]
    fn data_persists_across_open() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("access.db");

        // First open: create a group and assign a peer
        {
            let db = AccessDb::open(&path).unwrap();
            let peer = fake_peer(60);
            db.assign_peer_to_group(&peer, &GROUP_FRIENDS).unwrap();
            let _custom = db.create_group("persist-test", None, &[]).unwrap();
        }

        // Second open: verify data is still there
        {
            let db = AccessDb::open(&path).unwrap();
            let peer = fake_peer(60);
            let groups = db.list_peer_groups(&peer).unwrap();
            assert_eq!(groups.len(), 1);
            assert_eq!(groups[0].group_id, GROUP_FRIENDS);

            let all = db.list_groups().unwrap();
            assert_eq!(all.len(), 4); // 3 built-in + 1 custom
        }
    }
}
