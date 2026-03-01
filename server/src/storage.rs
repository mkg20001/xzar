use std::io::Write;
use std::path::{Path, PathBuf};
use tokio::fs::{self, File};
use tokio::io::{AsyncRead, AsyncWriteExt, BufWriter};

use crate::error::{AppError, Result};

/// File-based storage backend for NAR files
#[derive(Clone)]
pub struct Storage {
    pub base_path: PathBuf,
}

impl Storage {
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

    /// Write a file from an async reader, returning the number of bytes written
    pub async fn push<R: AsyncRead + Unpin>(
        &self,
        filename: &str,
        mut reader: R,
    ) -> Result<u64> {
        let path = self.file_path(filename);

        let file = File::create(&path).await?;
        let mut writer = BufWriter::new(file);

        let bytes_written = tokio::io::copy(&mut reader, &mut writer).await?;
        writer.flush().await?;

        Ok(bytes_written)
    }

    /// Write bytes to a file, computing SHA256 hash while writing
    /// Returns (bytes_written, sha256_hash_bytes)
    pub async fn push_with_hash<R: AsyncRead + Unpin>(
        &self,
        filename: &str,
        mut reader: R,
    ) -> Result<(u64, Vec<u8>)> {
        use sha2::{Digest, Sha256};

        let path = self.file_path(filename);
        let file = File::create(&path).await?;
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

        let hash = hasher.finalize().to_vec();
        Ok((total_bytes, hash))
    }

    /// Open a file for reading
    pub async fn pull(&self, filename: &str) -> Result<File> {
        let path = self.file_path(filename);

        if !path.exists() {
            return Err(AppError::NotFound(format!("File not found: {}", filename)));
        }

        let file = File::open(&path).await?;
        Ok(file)
    }

    /// Check if a file exists
    pub async fn exists(&self, filename: &str) -> bool {
        self.file_path(filename).exists()
    }

    /// Delete a file (idempotent - no error if file doesn't exist)
    pub async fn delete(&self, filename: &str) -> Result<()> {
        let path = self.file_path(filename);

        if path.exists() {
            fs::remove_file(&path).await?;
        }

        Ok(())
    }

    /// Get file size
    pub async fn size(&self, filename: &str) -> Result<u64> {
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
