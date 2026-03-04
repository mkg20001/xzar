use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use reqwest::multipart::{Form, Part};
use reqwest::Client;
use serde::Deserialize;
use tokio::sync::Mutex;
use xzar_common::{
    AdminTokenResponse, AdminUserResponse, CheckRequest, CheckResponse, CreateTokenRequest,
    CreateTokenResponse, CreateUserRequest, FinalizePinRequest, LockRequest, LockResponse,
    PinResponse, UpdateUserRequest,
};

pub use xzar_common::format_duration;

#[derive(Debug, Deserialize)]
struct ErrorResponse {
    error: Option<String>,
    message: Option<String>,
}

#[derive(Clone)]
pub struct ApiClient {
    client: Client,
    base_url: String,
    key: String,
    lock: Arc<Mutex<Option<LockState>>>,
}

struct LockState {
    id: i32,
    _renewal_task: tokio::task::JoinHandle<()>,
}

impl ApiClient {
    pub fn new(base_url: &str, key: &str) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(300))
            .build()
            .context("Failed to create HTTP client")?;

        Ok(Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
            key: key.to_string(),
            lock: Arc::new(Mutex::new(None)),
        })
    }

    async fn handle_response<T: for<'de> Deserialize<'de>>(&self, response: reqwest::Response) -> Result<T> {
        let status = response.status();
        let body = response.text().await?;

        if !status.is_success() {
            // Try to parse error response
            if let Ok(err) = serde_json::from_str::<ErrorResponse>(&body) {
                let msg = err.message.or(err.error).unwrap_or_else(|| "Unknown error".to_string());
                return Err(anyhow!("Server error ({}): {}", status, msg));
            }
            return Err(anyhow!("Server error ({}): {}", status, body));
        }

        serde_json::from_str(&body)
            .with_context(|| format!("Failed to parse response: {}", body))
    }

    /// Check which paths are not in the cache
    pub async fn check(&self, paths: &[String]) -> Result<Vec<String>> {
        let response = self.client
            .post(format!("{}/check", self.base_url))
            .header("Authorization", format!("Bearer {}", self.key))
            .json(&CheckRequest { paths: paths.to_vec() })
            .send()
            .await
            .context("Failed to send check request")?;

        let result: CheckResponse = self.handle_response(response).await?;
        Ok(result.need)
    }

    /// Request an upload lock
    pub async fn request_lock(&self) -> Result<i32> {
        let response = self.client
            .post(format!("{}/lock/request", self.base_url))
            .header("Authorization", format!("Bearer {}", self.key))
            .send()
            .await
            .context("Failed to request lock")?;

        let result: LockResponse = self.handle_response(response).await?;
        let lock_id = result.lock;

        // Start lock renewal task
        let client = self.clone();
        let renewal_task = tokio::spawn(async move {
            client.lock_renewal_loop(lock_id).await;
        });

        let mut lock_guard = self.lock.lock().await;
        *lock_guard = Some(LockState {
            id: lock_id,
            _renewal_task: renewal_task,
        });

        Ok(lock_id)
    }

    async fn lock_renewal_loop(&self, lock_id: i32) {
        // Renew every hour
        let mut interval = tokio::time::interval(Duration::from_secs(3600));
        interval.tick().await; // Skip first immediate tick

        loop {
            interval.tick().await;
            if let Err(e) = self.extend_lock(lock_id).await {
                tracing::warn!("Failed to extend lock: {}", e);
            } else {
                tracing::debug!("Lock {} extended", lock_id);
            }
        }
    }

    /// Extend lock deadline
    async fn extend_lock(&self, lock_id: i32) -> Result<()> {
        let response = self.client
            .post(format!("{}/lock/extend", self.base_url))
            .header("Authorization", format!("Bearer {}", self.key))
            .json(&LockRequest { lock: lock_id })
            .send()
            .await
            .context("Failed to extend lock")?;

        let _: LockResponse = self.handle_response(response).await?;
        Ok(())
    }

    /// Release the lock
    pub async fn clear_lock(&self) -> Result<()> {
        let mut lock_guard = self.lock.lock().await;
        if let Some(lock_state) = lock_guard.take() {
            lock_state._renewal_task.abort();

            let response = self.client
                .post(format!("{}/lock/clear", self.base_url))
                .header("Authorization", format!("Bearer {}", self.key))
                .json(&LockRequest { lock: lock_state.id })
                .send()
                .await
                .context("Failed to clear lock")?;

            if !response.status().is_success() {
                tracing::warn!("Failed to clear lock, status: {}", response.status());
            }
        }
        Ok(())
    }

    /// Get current lock ID
    pub async fn get_lock_id(&self) -> Option<i32> {
        let lock_guard = self.lock.lock().await;
        lock_guard.as_ref().map(|s| s.id)
    }

    /// Upload a NAR file
    pub async fn upload_nar(
        &self,
        lock_id: i32,
        drv_full: &str,
        hash: &str,
        size: u64,
        deriver: &str,
        references: &[String],
        data: Vec<u8>,
    ) -> Result<()> {
        let mut form = Form::new()
            .text("lock", lock_id.to_string())
            .text("drvFull", drv_full.to_string())
            .text("hash", hash.to_string())
            .text("size", size.to_string())
            .text("deriver", deriver.to_string())
            .text("compression", "xz");

        // Add references
        for reference in references {
            form = form.text("references[]", reference.clone());
        }

        // Add file
        let part = Part::bytes(data)
            .file_name("nar.xz")
            .mime_str("application/x-xz")?;
        form = form.part("file", part);

        let response = self.client
            .put(format!("{}/uploadNar", self.base_url))
            .header("Authorization", format!("Bearer {}", self.key))
            .multipart(form)
            .send()
            .await
            .context("Failed to upload NAR")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow!("Upload failed ({}): {}", status, body));
        }

        Ok(())
    }

    /// Finalize pin creation
    pub async fn finalize_pin(
        &self,
        name: &str,
        desc: Option<&str>,
        roots: &[String],
        expires: Option<u64>,
        leave_after_abandon: Option<u64>,
    ) -> Result<i32> {
        // Clear lock before finalizing
        self.clear_lock().await?;

        let request = FinalizePinRequest {
            roots: roots.to_vec(),
            name: name.to_string(),
            desc: desc.map(|s| s.to_string()),
            expires,
            leave_after_abandon,
        };

        let response = self.client
            .post(format!("{}/finalizePin", self.base_url))
            .header("Authorization", format!("Bearer {}", self.key))
            .json(&request)
            .send()
            .await
            .context("Failed to finalize pin")?;

        let pin_id: i32 = self.handle_response(response).await?;
        Ok(pin_id)
    }

    /// List all pins
    pub async fn list_pins(&self) -> Result<Vec<PinResponse>> {
        let response = self
            .client
            .get(format!("{}/pins", self.base_url))
            .header("Authorization", format!("Bearer {}", self.key))
            .send()
            .await
            .context("Failed to list pins")?;

        self.handle_response(response).await
    }

    // ============ Admin User API ============

    /// List all users (admin only)
    pub async fn list_users(&self) -> Result<Vec<AdminUserResponse>> {
        let response = self
            .client
            .get(format!("{}/admin/users", self.base_url))
            .header("Authorization", format!("Bearer {}", self.key))
            .send()
            .await
            .context("Failed to list users")?;

        self.handle_response(response).await
    }

    /// Create a new user (admin only)
    pub async fn create_user(&self, name: &str, is_admin: bool) -> Result<AdminUserResponse> {
        let request = CreateUserRequest {
            name: name.to_string(),
            is_admin,
        };

        let response = self
            .client
            .post(format!("{}/admin/users", self.base_url))
            .header("Authorization", format!("Bearer {}", self.key))
            .json(&request)
            .send()
            .await
            .context("Failed to create user")?;

        self.handle_response(response).await
    }

    /// Update a user (admin only)
    pub async fn update_user(
        &self,
        user_id: i32,
        name: Option<String>,
        is_admin: Option<bool>,
        email: Option<Option<String>>,
    ) -> Result<AdminUserResponse> {
        let request = UpdateUserRequest {
            name,
            is_admin,
            email,
        };

        let response = self
            .client
            .patch(format!("{}/admin/users/{}", self.base_url, user_id))
            .header("Authorization", format!("Bearer {}", self.key))
            .json(&request)
            .send()
            .await
            .context("Failed to update user")?;

        self.handle_response(response).await
    }

    /// Delete a user (admin only)
    pub async fn delete_user(&self, user_id: i32) -> Result<()> {
        let response = self
            .client
            .delete(format!("{}/admin/users/{}", self.base_url, user_id))
            .header("Authorization", format!("Bearer {}", self.key))
            .send()
            .await
            .context("Failed to delete user")?;

        let _: bool = self.handle_response(response).await?;
        Ok(())
    }

    // ============ Admin Token API ============

    /// List all tokens (admin only)
    pub async fn list_tokens(&self) -> Result<Vec<AdminTokenResponse>> {
        let response = self
            .client
            .get(format!("{}/admin/tokens", self.base_url))
            .header("Authorization", format!("Bearer {}", self.key))
            .send()
            .await
            .context("Failed to list tokens")?;

        self.handle_response(response).await
    }

    /// Create a new token (admin only)
    pub async fn create_token(
        &self,
        user_id: Option<i32>,
        description: Option<&str>,
    ) -> Result<CreateTokenResponse> {
        let request = CreateTokenRequest {
            user_id,
            description: description.map(|s| s.to_string()),
        };

        let response = self
            .client
            .post(format!("{}/admin/tokens", self.base_url))
            .header("Authorization", format!("Bearer {}", self.key))
            .json(&request)
            .send()
            .await
            .context("Failed to create token")?;

        self.handle_response(response).await
    }

    /// Delete/revoke a token (admin only)
    pub async fn delete_token(&self, token_id: i32) -> Result<()> {
        let response = self
            .client
            .delete(format!("{}/admin/tokens/{}", self.base_url, token_id))
            .header("Authorization", format!("Bearer {}", self.key))
            .send()
            .await
            .context("Failed to delete token")?;

        let _: bool = self.handle_response(response).await?;
        Ok(())
    }
}
