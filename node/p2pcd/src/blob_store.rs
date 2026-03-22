// Content-addressed blob storage.
//
// Blobs are stored by SHA-256 hash under <root>/blobs/<first-2-hex>/<full-hex>.
// No metadata database — the filesystem is the index.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use tokio::fs;
use tokio::io::AsyncWriteExt;

/// Content-addressed blob store backed by the filesystem.
pub struct BlobStore {
    root: PathBuf,
}

impl BlobStore {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            root: data_dir.join("blobs"),
        }
    }

    /// Check if a blob with the given hash exists.
    pub async fn has(&self, hash: &[u8; 32]) -> bool {
        self.path_for(hash).exists()
    }

    /// Get the size of a stored blob (None if not found).
    pub async fn size(&self, hash: &[u8; 32]) -> Option<u64> {
        fs::metadata(self.path_for(hash))
            .await
            .ok()
            .map(|m| m.len())
    }

    /// Read a chunk of a stored blob.
    pub async fn read_chunk(&self, hash: &[u8; 32], offset: u64, len: u64) -> Result<Vec<u8>> {
        use tokio::io::AsyncReadExt;

        let path = self.path_for(hash);
        let mut file = fs::File::open(&path)
            .await
            .with_context(|| format!("open blob {}", hex::encode(hash)))?;

        // Seek to offset
        if offset > 0 {
            use tokio::io::AsyncSeekExt;
            file.seek(std::io::SeekFrom::Start(offset)).await?;
        }

        let mut buf = vec![0u8; len as usize];
        let n = file.read(&mut buf).await?;
        buf.truncate(n);
        Ok(buf)
    }

    /// Begin writing a new blob. Returns a writer that accumulates data.
    /// Call `finalize()` to verify the hash and move to the final path.
    pub fn begin_write(&self, expected_hash: [u8; 32]) -> BlobWriter {
        let hex_hash = hex::encode(expected_hash);
        let temp_path = self.root.join("tmp").join(&hex_hash);
        BlobWriter {
            expected_hash,
            temp_path,
            final_path: self.path_for(&expected_hash),
            hasher: Sha256::new(),
            bytes_written: 0,
            file: None,
        }
    }

    /// Filesystem path for a blob by hash.
    fn path_for(&self, hash: &[u8; 32]) -> PathBuf {
        let hex_hash = hex::encode(hash);
        let prefix = &hex_hash[..2];
        self.root.join(prefix).join(&hex_hash)
    }
}

/// Accumulates blob data and verifies the hash on finalize.
pub struct BlobWriter {
    expected_hash: [u8; 32],
    temp_path: PathBuf,
    final_path: PathBuf,
    hasher: Sha256,
    bytes_written: u64,
    file: Option<fs::File>,
}

impl BlobWriter {
    /// Write a chunk of data.
    pub async fn write(&mut self, data: &[u8]) -> Result<()> {
        // Lazily create temp file and parent dirs
        if self.file.is_none() {
            if let Some(parent) = self.temp_path.parent() {
                fs::create_dir_all(parent).await?;
            }
            self.file = Some(
                fs::File::create(&self.temp_path)
                    .await
                    .context("create temp blob file")?,
            );
        }

        let file = self.file.as_mut().unwrap();
        file.write_all(data).await?;
        self.hasher.update(data);
        self.bytes_written += data.len() as u64;
        Ok(())
    }

    /// Verify the hash and move to the final content-addressed path.
    /// Returns the number of bytes written.
    pub async fn finalize(mut self) -> Result<u64> {
        if let Some(mut f) = self.file.take() {
            f.flush().await?;
        }

        // Verify hash
        let computed: [u8; 32] = self.hasher.finalize().into();
        if computed != self.expected_hash {
            // Clean up temp file
            let _ = fs::remove_file(&self.temp_path).await;
            bail!(
                "blob hash mismatch: expected {}, got {}",
                hex::encode(self.expected_hash),
                hex::encode(computed)
            );
        }

        // Move to final path
        if let Some(parent) = self.final_path.parent() {
            fs::create_dir_all(parent).await?;
        }
        fs::rename(&self.temp_path, &self.final_path)
            .await
            .context("move blob to final path")?;

        Ok(self.bytes_written)
    }

    /// Cancel the write and clean up the temp file.
    pub async fn cancel(mut self) {
        self.file.take(); // drop the file handle
        let _ = fs::remove_file(&self.temp_path).await;
    }

    pub fn bytes_written(&self) -> u64 {
        self.bytes_written
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    fn hash_data(data: &[u8]) -> [u8; 32] {
        let mut h = Sha256::new();
        h.update(data);
        h.finalize().into()
    }

    #[tokio::test]
    async fn store_and_retrieve() {
        let dir = tempfile::tempdir().unwrap();
        let store = BlobStore::new(dir.path());

        let data = b"hello blob world";
        let hash = hash_data(data);

        assert!(!store.has(&hash).await);

        let mut writer = store.begin_write(hash);
        writer.write(data).await.unwrap();
        writer.finalize().await.unwrap();

        assert!(store.has(&hash).await);
        assert_eq!(store.size(&hash).await, Some(data.len() as u64));

        let chunk = store.read_chunk(&hash, 0, 5).await.unwrap();
        assert_eq!(chunk, b"hello");

        let chunk2 = store.read_chunk(&hash, 6, 4).await.unwrap();
        assert_eq!(chunk2, b"blob");
    }

    #[tokio::test]
    async fn hash_mismatch_rejects() {
        let dir = tempfile::tempdir().unwrap();
        let store = BlobStore::new(dir.path());

        let data = b"some data";
        let wrong_hash = [0xAAu8; 32];

        let mut writer = store.begin_write(wrong_hash);
        writer.write(data).await.unwrap();
        let result = writer.finalize().await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("hash mismatch"));

        // Temp file should be cleaned up
        assert!(!store.has(&wrong_hash).await);
    }

    #[tokio::test]
    async fn dedup_same_hash() {
        let dir = tempfile::tempdir().unwrap();
        let store = BlobStore::new(dir.path());

        let data = b"dedup test";
        let hash = hash_data(data);

        // Write once
        let mut w1 = store.begin_write(hash);
        w1.write(data).await.unwrap();
        w1.finalize().await.unwrap();

        // Write again — should overwrite (same content, same path)
        let mut w2 = store.begin_write(hash);
        w2.write(data).await.unwrap();
        w2.finalize().await.unwrap();

        assert!(store.has(&hash).await);
        assert_eq!(store.size(&hash).await, Some(data.len() as u64));
    }

    #[tokio::test]
    async fn multi_chunk_write() {
        let dir = tempfile::tempdir().unwrap();
        let store = BlobStore::new(dir.path());

        let chunks: Vec<&[u8]> = vec![b"chunk1-", b"chunk2-", b"chunk3"];
        let full: Vec<u8> = chunks.iter().flat_map(|c| c.iter()).copied().collect();
        let hash = hash_data(&full);

        let mut writer = store.begin_write(hash);
        for chunk in &chunks {
            writer.write(chunk).await.unwrap();
        }
        let written = writer.finalize().await.unwrap();
        assert_eq!(written, full.len() as u64);

        let retrieved = store.read_chunk(&hash, 0, full.len() as u64).await.unwrap();
        assert_eq!(retrieved, full);
    }

    #[tokio::test]
    async fn cancel_cleans_up() {
        let dir = tempfile::tempdir().unwrap();
        let store = BlobStore::new(dir.path());

        let hash = [0xBBu8; 32];
        let mut writer = store.begin_write(hash);
        writer.write(b"partial data").await.unwrap();
        writer.cancel().await;

        assert!(!store.has(&hash).await);
    }
}
