use anyhow::{anyhow, Result};
use serde::Serialize;
use serde_json::Value as JsonValue;

#[derive(Debug, Clone, Serialize)]
pub struct WalrusConfig {
    pub enabled: bool,
    pub network: String,
    pub publisher_url: Option<String>,
    pub aggregator_url: Option<String>,
    pub epochs: u32,
    pub max_upload_bytes: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct WalrusStoredBlob {
    pub blob_id: String,
    pub object_id: Option<String>,
    pub size_bytes: usize,
    pub aggregator_url: Option<String>,
}

pub struct WalrusService {
    config: WalrusConfig,
    client: reqwest::Client,
}

impl WalrusConfig {
    pub fn from_env() -> Self {
        let publisher_url = non_empty_env("WALRUS_PUBLISHER_URL");
        let aggregator_url = non_empty_env("WALRUS_AGGREGATOR_URL");
        let enabled = std::env::var("WALRUS_ENABLED")
            .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
            .unwrap_or_else(|_| publisher_url.is_some());
        let epochs = std::env::var("WALRUS_EPOCHS")
            .ok()
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(5);
        let max_upload_bytes = std::env::var("WALRUS_MAX_UPLOAD_BYTES")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(1_048_576);

        Self {
            enabled,
            network: std::env::var("WALRUS_NETWORK").unwrap_or_else(|_| "testnet".to_string()),
            publisher_url,
            aggregator_url,
            epochs,
            max_upload_bytes,
        }
    }

    pub fn configured(&self) -> bool {
        self.enabled && self.publisher_url.is_some()
    }
}

impl WalrusService {
    pub fn new(config: WalrusConfig) -> Self {
        Self {
            config,
            client: reqwest::Client::new(),
        }
    }

    pub fn config(&self) -> &WalrusConfig {
        &self.config
    }

    pub fn blob_url(&self, blob_id: &str) -> Option<String> {
        self.config
            .aggregator_url
            .as_ref()
            .map(|base| format!("{}/v1/blobs/{}", base.trim_end_matches('/'), blob_id))
    }

    pub async fn store_json(&self, manifest: &JsonValue) -> Result<WalrusStoredBlob> {
        self.store_json_with_epochs(manifest, self.config.epochs)
            .await
    }

    /// Store JSON pinned for an explicit number of epochs. Used to keep
    /// high-stakes evidence available longer than the default retention.
    pub async fn store_json_with_epochs(
        &self,
        manifest: &JsonValue,
        epochs: u32,
    ) -> Result<WalrusStoredBlob> {
        if !self.config.enabled {
            return Err(anyhow!("Walrus storage is disabled"));
        }
        let Some(publisher_url) = self.config.publisher_url.as_ref() else {
            return Err(anyhow!("WALRUS_PUBLISHER_URL is not configured"));
        };

        let epochs = epochs.max(1);
        let body = serde_json::to_vec(manifest)?;
        if body.len() > self.config.max_upload_bytes {
            return Err(anyhow!(
                "Walrus artifact exceeds max size of {} bytes",
                self.config.max_upload_bytes
            ));
        }

        let url = format!(
            "{}/v1/blobs?epochs={}",
            publisher_url.trim_end_matches('/'),
            epochs
        );
        let response = self
            .client
            .put(url)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body.clone())
            .send()
            .await?;

        let status = response.status();
        let payload: JsonValue = response.json().await?;
        if !status.is_success() {
            return Err(anyhow!("Walrus publisher returned {}: {}", status, payload));
        }

        let blob_id = find_string_path(
            &payload,
            &[
                &["newlyCreated", "blobObject", "blobId"],
                &["alreadyCertified", "blobId"],
                &["blobId"],
            ],
        )
        .ok_or_else(|| anyhow!("Walrus response did not include a blob id: {}", payload))?;
        let object_id = find_string_path(
            &payload,
            &[
                &["newlyCreated", "blobObject", "id"],
                &["alreadyCertified", "event", "blobObjectId"],
                &["objectId"],
            ],
        );

        Ok(WalrusStoredBlob {
            aggregator_url: self.blob_url(&blob_id),
            blob_id,
            object_id,
            size_bytes: body.len(),
        })
    }
}

fn non_empty_env(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn find_string_path(value: &JsonValue, paths: &[&[&str]]) -> Option<String> {
    for path in paths {
        let mut cursor = value;
        for segment in *path {
            cursor = cursor.get(*segment)?;
        }
        if let Some(found) = cursor.as_str() {
            return Some(found.to_string());
        }
    }
    None
}
