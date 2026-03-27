//! User profile — name, bio, avatar, and homepage.
//!
//! Stored under `<data_dir>/profile/`:
//!   - `profile.json` — metadata
//!   - `avatar.{png,jpg,webp}` — profile picture
//!   - `homepage/` — user-provided HTML page + assets

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tracing::debug;

/// Maximum avatar file size: 1 MB.
pub const MAX_AVATAR_SIZE: usize = 1_024 * 1_024;

/// Allowed avatar extensions.
const AVATAR_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "webp"];

/// On-disk profile metadata.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Profile {
    /// Display name (synced from identity on first load).
    pub name: String,
    /// Short bio / status text (~280 chars).
    #[serde(default)]
    pub bio: String,
    /// Filename of the avatar image (relative to profile dir), e.g. "avatar.png".
    #[serde(default)]
    pub avatar: Option<String>,
    /// Path to the homepage index.html (relative to profile dir), e.g. "homepage/index.html".
    #[serde(default)]
    pub homepage: Option<String>,
}

/// Resolved paths for profile assets.
pub struct ProfilePaths {
    pub dir: PathBuf,
    pub json: PathBuf,
    pub homepage_dir: PathBuf,
}

impl ProfilePaths {
    pub fn new(data_dir: &Path) -> Self {
        let dir = data_dir.join("profile");
        Self {
            json: dir.join("profile.json"),
            homepage_dir: dir.join("homepage"),
            dir,
        }
    }
}

/// Load profile from disk, or create a default one seeded from identity name.
pub fn load_or_create(data_dir: &Path, identity_name: &str) -> anyhow::Result<Profile> {
    let paths = ProfilePaths::new(data_dir);
    std::fs::create_dir_all(&paths.dir)?;

    if paths.json.exists() {
        let text = std::fs::read_to_string(&paths.json)?;
        let profile: Profile = serde_json::from_str(&text)?;
        debug!("Profile loaded: {}", profile.name);
        return Ok(profile);
    }

    // Seed from identity
    let profile = Profile {
        name: identity_name.to_string(),
        bio: String::new(),
        avatar: None,
        homepage: None,
    };
    save(data_dir, &profile)?;
    debug!("Profile created for {}", profile.name);
    Ok(profile)
}

/// Persist profile to disk (atomic write).
pub fn save(data_dir: &Path, profile: &Profile) -> anyhow::Result<()> {
    let paths = ProfilePaths::new(data_dir);
    std::fs::create_dir_all(&paths.dir)?;
    let tmp = paths.dir.join("profile.json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(profile)?)?;
    std::fs::rename(&tmp, &paths.json)?;
    Ok(())
}

/// Validate and save an avatar image. Returns the filename stored.
///
/// Enforces size limit and extension whitelist.
pub fn save_avatar(data_dir: &Path, filename: &str, data: &[u8]) -> anyhow::Result<String> {
    if data.len() > MAX_AVATAR_SIZE {
        anyhow::bail!(
            "Avatar too large: {} bytes (max {})",
            data.len(),
            MAX_AVATAR_SIZE
        );
    }

    let ext = Path::new(filename)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();

    if !AVATAR_EXTENSIONS.contains(&ext.as_str()) {
        anyhow::bail!(
            "Invalid avatar format '{}' — allowed: {}",
            ext,
            AVATAR_EXTENSIONS.join(", ")
        );
    }

    let paths = ProfilePaths::new(data_dir);
    std::fs::create_dir_all(&paths.dir)?;

    // Remove any existing avatar files
    for existing_ext in AVATAR_EXTENSIONS {
        let existing = paths.dir.join(format!("avatar.{}", existing_ext));
        if existing.exists() {
            let _ = std::fs::remove_file(&existing);
        }
    }

    let stored_name = format!("avatar.{}", ext);
    let dest = paths.dir.join(&stored_name);
    std::fs::write(&dest, data)?;
    debug!("Avatar saved: {} ({} bytes)", stored_name, data.len());

    Ok(stored_name)
}

/// Read the avatar file bytes + content type. Returns None if no avatar set.
pub fn read_avatar(data_dir: &Path, profile: &Profile) -> Option<(Vec<u8>, &'static str)> {
    let avatar_name = profile.avatar.as_deref()?;
    let path = ProfilePaths::new(data_dir).dir.join(avatar_name);
    let data = std::fs::read(&path).ok()?;

    let content_type = match Path::new(avatar_name).extension().and_then(|e| e.to_str()) {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        _ => "application/octet-stream",
    };

    Some((data, content_type))
}

/// Resolve the homepage index file path. Returns None if not configured or missing.
pub fn resolve_homepage(data_dir: &Path, profile: &Profile) -> Option<PathBuf> {
    let homepage_rel = profile.homepage.as_deref()?;
    let paths = ProfilePaths::new(data_dir);
    let resolved = paths.dir.join(homepage_rel);

    // Security: ensure the resolved path is within the profile directory
    let canonical_dir = paths.dir.canonicalize().ok()?;
    let canonical_file = resolved.canonicalize().ok()?;
    if !canonical_file.starts_with(&canonical_dir) {
        tracing::warn!("Homepage path escapes profile dir: {}", resolved.display());
        return None;
    }

    if resolved.exists() {
        Some(resolved)
    } else {
        None
    }
}

/// Resolve a homepage asset path (css, images, etc). Returns None if outside homepage dir.
pub fn resolve_homepage_asset(
    data_dir: &Path,
    profile: &Profile,
    asset_path: &str,
) -> Option<PathBuf> {
    let homepage_rel = profile.homepage.as_deref()?;
    let paths = ProfilePaths::new(data_dir);

    // Homepage dir is the parent of the index.html
    let homepage_index = paths.dir.join(homepage_rel);
    let homepage_dir = homepage_index.parent()?;
    let resolved = homepage_dir.join(asset_path);

    // Security: ensure the resolved path stays within the homepage directory
    let canonical_dir = homepage_dir.canonicalize().ok()?;
    let canonical_file = resolved.canonicalize().ok()?;
    if !canonical_file.starts_with(&canonical_dir) {
        tracing::warn!("Homepage asset escapes directory: {}", resolved.display());
        return None;
    }

    if resolved.exists() {
        Some(resolved)
    } else {
        None
    }
}
