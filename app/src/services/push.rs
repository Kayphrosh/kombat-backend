// app/src/services/push.rs
//! Expo Push Notification service.
//! Sends push notifications via the Expo Push API.

use anyhow::Result;
use serde_json::Value as JsonValue;

const EXPO_PUSH_URL: &str = "https://exp.host/--/api/v2/push/send";

/// Send a push notification to one or more Expo push tokens.
pub async fn send_expo_push(
    tokens: &[String],
    title: &str,
    body: &str,
    data: Option<JsonValue>,
) -> Result<()> {
    let client = reqwest::Client::new();

    // Expo accepts an array of messages
    let messages: Vec<JsonValue> = tokens
        .iter()
        .map(|token| {
            let mut msg = serde_json::json!({
                "to": token,
                "sound": "default",
                "title": title,
                "body": body,
            });
            if let Some(ref d) = data {
                msg["data"] = d.clone();
            }
            msg
        })
        .collect();

    let response = client
        .post(EXPO_PUSH_URL)
        .header("Content-Type", "application/json")
        .json(&messages)
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        tracing::error!("Expo push failed ({}): {}", status, text);
        anyhow::bail!("Expo push failed: {} - {}", status, text);
    }

    tracing::info!("Expo push sent to {} tokens", tokens.len());
    Ok(())
}
