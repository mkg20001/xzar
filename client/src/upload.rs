use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use futures::stream::{self, StreamExt};
use indicatif::ProgressBar;
use tokio::sync::Semaphore;

use crate::api::ApiClient;
use crate::nix::NixStore;

const MAX_UPLOAD_RETRIES: u32 = 3;
const RETRY_BASE_DELAY: Duration = Duration::from_secs(5);

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

                    let basename = NixStore::basename(&path);

                    // Get path details (doesn't need to be retried)
                    let details = nix.get_details(&path).await
                        .with_context(|| format!("Failed to get details for {}", path))?;

                    // Retry loop for upload (stream body can't be replayed, so we
                    // restart the nix-store --dump → compress pipeline on each attempt)
                    let mut last_err = None;
                    for attempt in 0..=MAX_UPLOAD_RETRIES {
                        if attempt > 0 {
                            let delay = RETRY_BASE_DELAY * 2u32.pow(attempt - 1);
                            tracing::warn!(
                                "Retrying upload of {} (attempt {}/{}), waiting {:?}",
                                basename, attempt + 1, MAX_UPLOAD_RETRIES + 1, delay
                            );
                            tokio::time::sleep(delay).await;
                        }

                        progress.set_message(format!("uploading {}", basename));

                        // Start a fresh streaming compression pipeline
                        let body = match nix.dump_nar_stream(&path, use_pixz) {
                            Ok(body) => body,
                            Err(e) => {
                                last_err = Some(e.context(format!("Failed to start compression for {}", path)));
                                continue;
                            }
                        };

                        match api.upload_nar(
                            lock_id,
                            &basename,
                            &details.hash,
                            details.size,
                            &details.deriver,
                            &details.references,
                            body,
                        ).await {
                            Ok(()) => {
                                if attempt > 0 {
                                    tracing::info!("Upload of {} succeeded on attempt {}", basename, attempt + 1);
                                }
                                progress.inc(1);
                                return Ok(());
                            }
                            Err(e) => {
                                tracing::warn!("Upload of {} failed: {:#}", basename, e);
                                last_err = Some(e);
                            }
                        }
                    }

                    Err(last_err
                        .unwrap_or_else(|| anyhow::anyhow!("Upload failed"))
                        .context(format!("Failed to upload {} after {} attempts", path, MAX_UPLOAD_RETRIES + 1)))
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
