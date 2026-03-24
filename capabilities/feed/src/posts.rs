// Post and attachment types + validation.
//
// All IO has moved to db.rs. This module defines the data structures and
// validation logic only.

use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

// ── Attachment ───────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Attachment {
    /// SHA-256 hash of the blob, hex-encoded.
    pub blob_id: String,
    /// MIME type: image/jpeg, image/png, image/webp, image/gif, video/mp4.
    pub mime_type: String,
    /// Full blob size in bytes.
    pub size: u64,
}

// ── Post ─────────────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Post {
    pub id: String,
    pub author_id: String,
    pub author_name: String,
    pub content: String,
    pub timestamp: u64,
    /// Where this post came from: "local" for our own, "peer:<b64_peer_id>" for received.
    #[serde(default = "default_origin")]
    pub origin: String,
    /// Media attachments (images, GIFs, video). Optional for backward compat.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<Attachment>,
}

fn default_origin() -> String {
    "local".to_string()
}

// ── Validation ───────────────────────────────────────────────────────────────

pub const MAX_CONTENT_LEN: usize = 5000;

// ── Configurable media limits ────────────────────────────────────────────────

/// Default limits (used when no config override is provided).
pub const DEFAULT_MAX_ATTACHMENTS: usize = 4;
pub const DEFAULT_MAX_IMAGE_SIZE: u64 = 8 * 1024 * 1024; // 8 MB
pub const DEFAULT_MAX_VIDEO_SIZE: u64 = 50 * 1024 * 1024; // 50 MB

/// Allowed MIME types for attachments.
pub const ALLOWED_IMAGE_MIMES: &[&str] = &["image/jpeg", "image/png", "image/webp", "image/gif"];
pub const ALLOWED_VIDEO_MIMES: &[&str] = &["video/mp4", "video/webm"];

/// All allowed MIME types.
pub fn allowed_mime_types() -> Vec<&'static str> {
    let mut v: Vec<&str> = ALLOWED_IMAGE_MIMES.to_vec();
    v.extend_from_slice(ALLOWED_VIDEO_MIMES);
    v
}

/// Configurable media limits for the social feed.
/// Exposed via `GET /post/limits` so the UI can enforce client-side.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaLimits {
    pub max_attachments: usize,
    pub max_image_bytes: u64,
    pub max_video_bytes: u64,
    pub allowed_mime_types: Vec<String>,
}

impl Default for MediaLimits {
    fn default() -> Self {
        Self {
            max_attachments: DEFAULT_MAX_ATTACHMENTS,
            max_image_bytes: DEFAULT_MAX_IMAGE_SIZE,
            max_video_bytes: DEFAULT_MAX_VIDEO_SIZE,
            allowed_mime_types: allowed_mime_types().into_iter().map(String::from).collect(),
        }
    }
}

/// Backward-compat aliases for tests that reference the old constants.
pub const MAX_ATTACHMENTS: usize = DEFAULT_MAX_ATTACHMENTS;
pub const MAX_IMAGE_SIZE: u64 = DEFAULT_MAX_IMAGE_SIZE;
pub const MAX_VIDEO_SIZE: u64 = DEFAULT_MAX_VIDEO_SIZE;
pub const ALLOWED_MIME_TYPES: &[&str] = &[
    "image/jpeg",
    "image/png",
    "image/webp",
    "image/gif",
    "video/mp4",
    "video/webm",
];

#[derive(Debug, Serialize)]
pub struct AttachmentError {
    pub index: usize,
    pub constraint: String,
    pub message: String,
}

/// Validate attachment metadata. Returns a list of errors (empty = valid).
pub fn validate_attachments(attachments: &[Attachment]) -> Vec<AttachmentError> {
    let mut errors = Vec::new();

    if attachments.len() > MAX_ATTACHMENTS {
        errors.push(AttachmentError {
            index: 0,
            constraint: "max_count".to_string(),
            message: format!("too many attachments (max {})", MAX_ATTACHMENTS),
        });
        return errors; // no point checking individual attachments
    }

    for (i, att) in attachments.iter().enumerate() {
        if !ALLOWED_MIME_TYPES.contains(&att.mime_type.as_str()) {
            errors.push(AttachmentError {
                index: i,
                constraint: "mime_type".to_string(),
                message: format!("unsupported MIME type: {}", att.mime_type),
            });
        }

        let max = if att.mime_type == "video/mp4" {
            MAX_VIDEO_SIZE
        } else {
            MAX_IMAGE_SIZE
        };
        if att.size > max {
            errors.push(AttachmentError {
                index: i,
                constraint: "max_size".to_string(),
                message: format!("attachment too large ({} bytes, max {})", att.size, max),
            });
        }
    }

    errors
}

/// Build a new local post with a fresh UUID and current timestamp.
/// Pass empty vec for text-only posts.
pub fn new_post(
    content: String,
    author_id: String,
    author_name: String,
    attachments: Vec<Attachment>,
) -> anyhow::Result<Post> {
    if content.len() > MAX_CONTENT_LEN {
        return Err(anyhow::anyhow!(
            "content too long (max {} chars)",
            MAX_CONTENT_LEN
        ));
    }
    Ok(Post {
        id: Uuid::new_v4().to_string(),
        author_id,
        author_name,
        content,
        timestamp: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
        origin: "local".to_string(),
        attachments,
    })
}

/// Validate attachments against configurable limits.
pub fn validate_attachments_with_limits(
    attachments: &[Attachment],
    limits: &MediaLimits,
) -> Vec<AttachmentError> {
    let mut errors = Vec::new();

    if attachments.len() > limits.max_attachments {
        errors.push(AttachmentError {
            index: 0,
            constraint: "max_count".to_string(),
            message: format!("too many attachments (max {})", limits.max_attachments),
        });
        return errors;
    }

    let allowed: Vec<&str> = limits
        .allowed_mime_types
        .iter()
        .map(|s| s.as_str())
        .collect();

    for (i, att) in attachments.iter().enumerate() {
        if !allowed.contains(&att.mime_type.as_str()) {
            errors.push(AttachmentError {
                index: i,
                constraint: "mime_type".to_string(),
                message: format!("unsupported MIME type: {}", att.mime_type),
            });
        }

        let is_video = ALLOWED_VIDEO_MIMES.contains(&att.mime_type.as_str());
        let max = if is_video {
            limits.max_video_bytes
        } else {
            limits.max_image_bytes
        };
        if att.size > max {
            errors.push(AttachmentError {
                index: i,
                constraint: "max_size".to_string(),
                message: format!("attachment too large ({} bytes, max {})", att.size, max),
            });
        }
    }

    errors
}

/// Prepare a peer post for ingestion: set origin if not already set,
/// validate content length.
pub fn prepare_peer_post(mut post: Post, peer_id_b64: &str) -> anyhow::Result<Post> {
    if post.content.len() > MAX_CONTENT_LEN {
        return Err(anyhow::anyhow!("peer post too long, rejected"));
    }
    if post.origin == "local" || post.origin.is_empty() {
        post.origin = format!("peer:{}", peer_id_b64);
    }
    Ok(post)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_post_basic() {
        let post = new_post("hello".into(), "alice".into(), "Alice".into(), vec![]).unwrap();
        assert_eq!(post.origin, "local");
        assert!(!post.id.is_empty());
        assert!(post.attachments.is_empty());
    }

    #[test]
    fn new_post_content_too_long() {
        let long = "x".repeat(MAX_CONTENT_LEN + 1);
        assert!(new_post(long, "a".into(), "A".into(), vec![]).is_err());
    }

    #[test]
    fn prepare_peer_post_sets_origin() {
        let post = Post {
            id: "test".into(),
            author_id: "bob".into(),
            author_name: "Bob".into(),
            content: "hi".into(),
            timestamp: 1000,
            origin: "local".into(),
            attachments: vec![],
        };
        let prepared = prepare_peer_post(post, "AAAA").unwrap();
        assert_eq!(prepared.origin, "peer:AAAA");
    }

    #[test]
    fn prepare_peer_post_rejects_long() {
        let post = Post {
            id: "test".into(),
            author_id: "bob".into(),
            author_name: "Bob".into(),
            content: "x".repeat(MAX_CONTENT_LEN + 1),
            timestamp: 1000,
            origin: "local".into(),
            attachments: vec![],
        };
        assert!(prepare_peer_post(post, "AAAA").is_err());
    }

    #[test]
    fn validate_attachments_all_ok() {
        let atts = vec![
            Attachment {
                blob_id: "aa".into(),
                mime_type: "image/jpeg".into(),
                size: 1000,
            },
            Attachment {
                blob_id: "bb".into(),
                mime_type: "video/mp4".into(),
                size: MAX_VIDEO_SIZE,
            },
        ];
        assert!(validate_attachments(&atts).is_empty());
    }

    #[test]
    fn validate_attachments_too_many() {
        let atts: Vec<Attachment> = (0..5)
            .map(|i| Attachment {
                blob_id: format!("{:02x}", i),
                mime_type: "image/jpeg".into(),
                size: 100,
            })
            .collect();
        let errs = validate_attachments(&atts);
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].constraint, "max_count");
    }

    #[test]
    fn validate_attachments_bad_mime() {
        let atts = vec![Attachment {
            blob_id: "aa".into(),
            mime_type: "image/bmp".into(),
            size: 100,
        }];
        let errs = validate_attachments(&atts);
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].constraint, "mime_type");
        assert_eq!(errs[0].index, 0);
    }

    #[test]
    fn validate_attachments_image_too_large() {
        let atts = vec![Attachment {
            blob_id: "aa".into(),
            mime_type: "image/png".into(),
            size: MAX_IMAGE_SIZE + 1,
        }];
        let errs = validate_attachments(&atts);
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].constraint, "max_size");
    }

    #[test]
    fn validate_attachments_video_boundary() {
        // Exactly at limit — ok
        let ok = vec![Attachment {
            blob_id: "aa".into(),
            mime_type: "video/mp4".into(),
            size: MAX_VIDEO_SIZE,
        }];
        assert!(validate_attachments(&ok).is_empty());

        // One byte over — fail
        let fail = vec![Attachment {
            blob_id: "aa".into(),
            mime_type: "video/mp4".into(),
            size: MAX_VIDEO_SIZE + 1,
        }];
        assert_eq!(validate_attachments(&fail).len(), 1);
    }

    #[test]
    fn validate_attachments_image_boundary() {
        let ok = vec![Attachment {
            blob_id: "aa".into(),
            mime_type: "image/jpeg".into(),
            size: MAX_IMAGE_SIZE,
        }];
        assert!(validate_attachments(&ok).is_empty());

        let fail = vec![Attachment {
            blob_id: "aa".into(),
            mime_type: "image/jpeg".into(),
            size: MAX_IMAGE_SIZE + 1,
        }];
        assert_eq!(validate_attachments(&fail).len(), 1);
    }

    #[test]
    fn backward_compat_deserialize() {
        // Old JSON without attachments or origin fields
        let json = r#"{"id":"old","author_id":"a","author_name":"A","content":"hi","timestamp":1}"#;
        let post: Post = serde_json::from_str(json).unwrap();
        assert_eq!(post.origin, "local");
        assert!(post.attachments.is_empty());
    }

    #[test]
    fn round_trip_with_attachments() {
        let post = Post {
            id: "p1".into(),
            author_id: "a".into(),
            author_name: "A".into(),
            content: "media post".into(),
            timestamp: 1000,
            origin: "local".into(),
            attachments: vec![
                Attachment {
                    blob_id: "aabb".into(),
                    mime_type: "image/jpeg".into(),
                    size: 1000,
                },
                Attachment {
                    blob_id: "ccdd".into(),
                    mime_type: "image/gif".into(),
                    size: 2000,
                },
                Attachment {
                    blob_id: "eeff".into(),
                    mime_type: "image/webp".into(),
                    size: 3000,
                },
                Attachment {
                    blob_id: "0011".into(),
                    mime_type: "video/mp4".into(),
                    size: 4000,
                },
            ],
        };
        let json = serde_json::to_string(&post).unwrap();
        let back: Post = serde_json::from_str(&json).unwrap();
        assert_eq!(back.attachments.len(), 4);
        assert_eq!(back.attachments[0].blob_id, "aabb");
        assert_eq!(back.attachments[3].mime_type, "video/mp4");
    }

    #[test]
    fn skip_serializing_empty_attachments() {
        let post = new_post("text only".into(), "a".into(), "A".into(), vec![]).unwrap();
        let json = serde_json::to_string(&post).unwrap();
        assert!(!json.contains("attachments"));
    }
}
