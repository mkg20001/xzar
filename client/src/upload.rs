use std::sync::Arc;

use anyhow::{Context, Result};
use futures::stream::{self, StreamExt};
use indicatif::ProgressBar;
use tokio::sync::Semaphore;

use crate::api::ApiClient;
use crate::nix::NixStore;

pub struct UploadManager {
    api: ApiClient,
    nix: NixStore,
    progress: ProgressBar,
    parallelism: usize,
    use_pixz: bool,
}

impl UploadManager {
    pub fn new(
        api: ApiClient,
        nix: NixStore,
        progress: ProgressBar,
        parallelism: usize,
        use_pixz: bool,
    ) -> Self {
        Self {
            api,
            nix,
            progress,
            parallelism,
            use_pixz,
        }
    }

    pub async fn upload_all(&mut self, paths: &[String]) -> Result<()> {
        // Request lock before starting uploads
        let lock_id = self.api.request_lock().await
            .context("Failed to acquire upload lock")?;

        // Create semaphore for controlling parallelism
        let semaphore = Arc::new(Semaphore::new(self.parallelism));

        // Pre-fetch buffer: allow up to 2x parallelism items to be prepared
        let buffer_size = self.parallelism * 2;

        // Create upload tasks
        let api = self.api.clone();
        let nix = self.nix.clone();
        let progress = self.progress.clone();
        let use_pixz = self.use_pixz;

        // Process all paths with maximum parallelism
        let results: Vec<Result<()>> = stream::iter(paths.iter().cloned())
            .map(|path| {
                let semaphore = semaphore.clone();
                let api = api.clone();
                let nix = nix.clone();
                let progress = progress.clone();

                async move {
                    // Acquire semaphore permit
                    let _permit = semaphore.acquire().await
                        .map_err(|e| anyhow::anyhow!("Semaphore error: {}", e))?;

                    // Update progress message
                    let basename = NixStore::basename(&path);
                    progress.set_message(format!("uploading {}", basename));

                    // Start streaming compression (spawns processes, returns immediately)
                    let body = nix.dump_nar_stream(&path, use_pixz)
                        .with_context(|| format!("Failed to start compression for {}", path))?;

                    // Get path details while compression streams
                    let details = nix.get_details(&path).await
                        .with_context(|| format!("Failed to get details for {}", path))?;

                    // Stream compressed data directly to server
                    api.upload_nar(
                        lock_id,
                        &basename,
                        &details.hash,
                        details.size,
                        &details.deriver,
                        &details.references,
                        body,
                    ).await
                        .with_context(|| format!("Failed to upload {}", path))?;

                    // Update progress
                    progress.inc(1);

                    Ok(())
                }
            })
            .buffer_unordered(buffer_size)
            .collect()
            .await;

        // Check for errors
        let mut errors = Vec::new();
        for result in results {
            if let Err(e) = result {
                errors.push(e);
            }
        }

        if !errors.is_empty() {
            // Clear lock on error
            let _ = self.api.clear_lock().await;

            // Report first error, log others
            for (i, err) in errors.iter().enumerate() {
                if i > 0 {
                    tracing::error!("Additional upload error: {}", err);
                }
            }
            return Err(errors.remove(0));
        }

        Ok(())
    }
}
