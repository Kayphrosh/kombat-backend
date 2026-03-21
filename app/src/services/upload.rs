

use anyhow::{anyhow, Result};
use std::path::PathBuf;

pub struct UploadService {
    upload_dir: PathBuf,
    base_url: String,
}

impl UploadService {
    pub fn new(upload_dir: &str, base_url: &str) -> Result<Self> {
        let path = PathBuf::from(upload_dir);
        std::fs::create_dir_all(&path)?;
        Ok(Self {
            upload_dir: path,
            base_url: base_url.trim_end_matches('/').to_string(),
        })
    }

    /// Save file bytes to disk and return the public URL.
    /// `file_type` is "avatar" or "evidence".
    pub async fn save_file(
        &self,
        data: Vec<u8>,
        original_filename: &str,
        file_type: &str,
    ) -> Result<String> {
        // Validate size (5MB limit)
        if data.len() > 5 * 1024 * 1024 {
            return Err(anyhow!("File too large (max 5MB)"));
        }

        // Extract extension from original filename
        let extension = original_filename
            .rsplit('.')
            .next()
            .unwrap_or("bin")
            .to_lowercase();

        // Validate image extensions for avatar/evidence
        let allowed = ["jpg", "jpeg", "png", "gif", "webp"];
        if !allowed.contains(&extension.as_str()) {
            return Err(anyhow!(
                "Invalid file type '{}'. Allowed: {}",
                extension,
                allowed.join(", ")
            ));
        }

        // Generate unique filename
        let filename = format!("{}_{}.{}", file_type, uuid::Uuid::new_v4(), extension);
        let filepath = self.upload_dir.join(&filename);

        // Write file asynchronously
        tokio::fs::write(&filepath, &data)
            .await
            .map_err(|e| anyhow!("Failed to write file: {}", e))?;

        let url = format!("{}/uploads/{}", self.base_url, filename);
        Ok(url)
    }
}
