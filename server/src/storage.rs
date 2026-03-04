use std::io::Write;
use std::path::{Path, PathBuf};
use std::pin::Pin;

use async_trait::async_trait;
use futures::Stream;
use tokio::fs::{self, File};
use tokio::io::{AsyncRead, AsyncWriteExt, BufWriter};

use crate::error::{AppError, Result};

/// A boxed async byte stream for reading from storage
pub type ByteStream = Pin<Box<dyn Stream<Item = std::io::Result<bytes::Bytes>> + Send>>;

/// Abstract storage backend trait
#[async_trait]
pub trait StorageBackend: Send + Sync + Clone {
    /// Write a file from an async reader, computing SHA256 hash while writing.
    /// Returns (bytes_written, sha256_hash_bytes).
    /// On failure, the destination file is automatically cleaned up.
    async fn push_with_hash<R: AsyncRead + Send + Unpin>(
        &self,
        filename: &str,
        reader: R,
    ) -> Result<(u64, Vec<u8>)>;

    /// Open a file for streamed reading, returns a byte stream
    async fn pull(&self, filename: &str) -> Result<ByteStream>;

    /// Check if a file exists
    async fn exists(&self, filename: &str) -> bool;

    /// Delete a file (idempotent - no error if file doesn't exist)
    async fn delete(&self, filename: &str) -> Result<()>;

    /// Get file size
    async fn size(&self, filename: &str) -> Result<u64>;
}

/// File-based storage backend for NAR files
#[derive(Clone)]
pub struct FileStorage {
    base_path: PathBuf,
}

impl FileStorage {
    pub async fn new<P: AsRef<Path>>(base_path: P) -> Result<Self> {
        let path = base_path.as_ref().to_path_buf();

        // Ensure the storage directory exists
        fs::create_dir_all(&path).await?;

        Ok(Self { base_path: path })
    }

    /// Get the full path for a file
    fn file_path(&self, filename: &str) -> PathBuf {
        self.base_path.join(filename)
    }
}

/// Guard that deletes a file on drop unless disarmed.
/// Used for cleanup on failed writes.
struct WriteGuard {
    path: PathBuf,
    armed: bool,
}

impl WriteGuard {
    fn new(path: PathBuf) -> Self {
        Self { path, armed: true }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for WriteGuard {
    fn drop(&mut self) {
        if self.armed {
            // Best-effort cleanup - ignore errors
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

#[async_trait]
impl StorageBackend for FileStorage {
    async fn push_with_hash<R: AsyncRead + Send + Unpin>(
        &self,
        filename: &str,
        mut reader: R,
    ) -> Result<(u64, Vec<u8>)> {
        use sha2::{Digest, Sha256};

        let path = self.file_path(filename);

        // Create the file
        let file = File::create(&path).await?;

        // Arm the cleanup guard - will delete file if we don't disarm
        let mut guard = WriteGuard::new(path);

        let mut writer = BufWriter::new(file);
        let mut hasher = Sha256::new();
        let mut total_bytes = 0u64;

        let mut buf = [0u8; 64 * 1024]; // 64KB buffer

        loop {
            let n = tokio::io::AsyncReadExt::read(&mut reader, &mut buf).await?;
            if n == 0 {
                break;
            }

            hasher.update(&buf[..n]);
            writer.write_all(&buf[..n]).await?;
            total_bytes += n as u64;
        }

        writer.flush().await?;

        // Success - disarm the guard so file is kept
        guard.disarm();

        let hash = hasher.finalize().to_vec();
        Ok((total_bytes, hash))
    }

    async fn pull(&self, filename: &str) -> Result<ByteStream> {
        use tokio_util::io::ReaderStream;

        let path = self.file_path(filename);

        if !path.exists() {
            return Err(AppError::NotFound(format!("File not found: {}", filename)));
        }

        let file = File::open(&path).await?;
        let stream = ReaderStream::new(file);

        Ok(Box::pin(stream))
    }

    async fn exists(&self, filename: &str) -> bool {
        self.file_path(filename).exists()
    }

    async fn delete(&self, filename: &str) -> Result<()> {
        let path = self.file_path(filename);

        if path.exists() {
            fs::remove_file(&path).await?;
        }

        Ok(())
    }

    async fn size(&self, filename: &str) -> Result<u64> {
        let path = self.file_path(filename);
        let metadata = fs::metadata(&path).await?;
        Ok(metadata.len())
    }
}

/// Synchronous helper for writing with hash computation
pub fn write_with_hash<W: Write, R: std::io::Read>(
    mut writer: W,
    mut reader: R,
) -> std::io::Result<(u64, Vec<u8>)> {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    let mut total_bytes = 0u64;
    let mut buf = [0u8; 64 * 1024];

    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }

        hasher.update(&buf[..n]);
        writer.write_all(&buf[..n])?;
        total_bytes += n as u64;
    }

    writer.flush()?;

    Ok((total_bytes, hasher.finalize().to_vec()))
}

// Re-export FileStorage as Storage for backwards compatibility
pub type Storage = FileStorage;
