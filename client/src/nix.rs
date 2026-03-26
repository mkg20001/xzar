use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::OnceLock;

use anyhow::{anyhow, Context, Result};
use async_compression::tokio::bufread::XzEncoder;
use tokio::io::{AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio_util::io::ReaderStream;

/// Check once whether pixz is available on PATH
fn pixz_available() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        let available = which::which("pixz").is_ok();
        if !available {
            tracing::warn!("pixz not found, falling back to in-process xz compression");
        }
        available
    })
}

/// Details about a Nix store path
#[derive(Debug, Clone)]
pub struct PathDetails {
    pub path: PathBuf,
    pub hash: String,
    pub size: u64,
    pub deriver: String,
    pub references: Vec<String>,
}

#[derive(Clone)]
pub struct NixStore;

impl NixStore {
    pub fn new() -> Self {
        Self
    }

    /// Get the transitive closure of store paths
    pub async fn get_closure(&self, paths: &[PathBuf]) -> Result<Vec<String>> {
        let path_strs: Vec<&str> = paths.iter()
            .filter_map(|p| p.to_str())
            .collect();

        let output = Command::new("nix-store")
            .arg("--query")
            .arg("--requisites")
            .args(&path_strs)
            .output()
            .await
            .context("Failed to run nix-store --query --requisites")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow!("nix-store --query --requisites failed: {}", stderr));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let paths: Vec<String> = stdout
            .lines()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        Ok(paths)
    }

    /// Get details for a store path
    pub async fn get_details(&self, path: &str) -> Result<PathDetails> {
        // Run all queries in parallel
        let (hash, size, deriver, references) = tokio::try_join!(
            self.query_hash(path),
            self.query_size(path),
            self.query_deriver(path),
            self.query_references(path),
        )?;

        Ok(PathDetails {
            path: PathBuf::from(path),
            hash,
            size,
            deriver,
            references,
        })
    }

    async fn query_hash(&self, path: &str) -> Result<String> {
        self.run_query(path, "--hash").await
    }

    async fn query_size(&self, path: &str) -> Result<u64> {
        let output = self.run_query(path, "--size").await?;
        output.parse().context("Failed to parse size")
    }

    async fn query_deriver(&self, path: &str) -> Result<String> {
        let result = self.run_query(path, "--deriver").await?;
        // Return basename only, handle "unknown-deriver" case
        if result == "unknown-deriver" || result.is_empty() {
            return Ok(String::new());
        }
        Ok(Path::new(&result)
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default())
    }

    async fn query_references(&self, path: &str) -> Result<Vec<String>> {
        let output = Command::new("nix-store")
            .arg("--query")
            .arg("--references")
            .arg(path)
            .output()
            .await
            .context("Failed to run nix-store --query --references")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow!("nix-store --query --references failed: {}", stderr));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let refs: Vec<String> = stdout
            .lines()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .filter_map(|s| Path::new(s).file_name())
            .map(|s| s.to_string_lossy().to_string())
            .collect();

        Ok(refs)
    }

    async fn run_query(&self, path: &str, flag: &str) -> Result<String> {
        // Retry logic for empty outputs
        for attempt in 0..3 {
            let output = Command::new("nix-store")
                .arg("--query")
                .arg(flag)
                .arg(path)
                .output()
                .await
                .context(format!("Failed to run nix-store --query {}", flag))?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(anyhow!("nix-store --query {} failed: {}", flag, stderr));
            }

            let result = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !result.is_empty() || attempt == 2 {
                return Ok(result);
            }

            // Retry on empty output
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        }

        Ok(String::new())
    }

    /// Spawn nix-store --dump and return the child process
    fn spawn_nar_dump(path: &str) -> Result<tokio::process::Child> {
        Command::new("nix-store")
            .arg("--dump")
            .arg(path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("Failed to spawn nix-store --dump")
    }

    /// Dump a store path as NAR and compress with xz, returning a streaming body.
    /// Data flows: nix-store --dump → compressor → HTTP upload without buffering.
    pub fn dump_nar_stream(&self, path: &str, use_pixz: bool) -> Result<reqwest::Body> {
        let mut nar_process = Self::spawn_nar_dump(path)?;
        let nar_stdout = nar_process.stdout.take()
            .ok_or_else(|| anyhow!("Failed to get stdout from nix-store"))?;

        if use_pixz && pixz_available() {
            Self::compress_stream_external(nar_stdout, "pixz")
        } else {
            Ok(Self::compress_stream_inprocess(nar_stdout))
        }
    }

    /// Stream compressed data from an external compressor (pixz)
    fn compress_stream_external(
        nar_stdout: tokio::process::ChildStdout,
        compressor: &str,
    ) -> Result<reqwest::Body> {
        let mut xz_process = Command::new(compressor)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context(format!("Failed to spawn {}", compressor))?;

        let mut xz_stdin = xz_process.stdin.take()
            .ok_or_else(|| anyhow!("Failed to get stdin for {}", compressor))?;
        let xz_stdout = xz_process.stdout.take()
            .ok_or_else(|| anyhow!("Failed to get stdout from {}", compressor))?;

        // Pipe nix-store stdout → compressor stdin in background
        tokio::spawn(async move {
            let mut nar_stdout = nar_stdout;
            if let Err(e) = tokio::io::copy(&mut nar_stdout, &mut xz_stdin).await {
                tracing::error!("Failed to pipe NAR to compressor: {}", e);
            }
            let _ = xz_stdin.shutdown().await;
        });

        // Stream compressor stdout directly to upload
        let stream = ReaderStream::new(xz_stdout);
        Ok(reqwest::Body::wrap_stream(stream))
    }

    /// Stream compressed data using in-process xz via async-compression
    fn compress_stream_inprocess(
        nar_stdout: tokio::process::ChildStdout,
    ) -> reqwest::Body {
        let encoder = XzEncoder::new(BufReader::new(nar_stdout));
        let stream = ReaderStream::new(encoder);
        reqwest::Body::wrap_stream(stream)
    }

    /// Get basename of a path
    pub fn basename(path: &str) -> String {
        Path::new(path)
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| path.to_string())
    }
}
