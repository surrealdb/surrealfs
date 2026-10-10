//! Webhook outbound dispatcher with HMAC signatures and exponential retry (§23.2).

use crate::errors::Result;
use crate::fs::SurrealFs;
use crate::models::{WebhookDeliveryRecord, WebhookRecord};
use chrono::Utc;
use hmac::{Hmac, Mac};
use reqwest::Client;
use serde_json::Value;
use sha2::Sha256;
use std::time::Duration;
use tracing::{debug, warn};

type HmacSha256 = Hmac<Sha256>;

/// Compute HMAC-SHA256 signature formatted as `sha256=<hex>`.
pub fn compute_signature(secret: &str, body: &[u8]) -> String {
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC can take key of any size");
    mac.update(body);
    let result = mac.finalize();
    format!("sha256={}", hex::encode(result.into_bytes()))
}

/// Outbound webhook dispatcher with retries, exponential backoff, and signature verification.
#[derive(Debug, Clone)]
pub struct WebhookDispatcher {
    client: Client,
    pub max_retries: u32,
    pub initial_backoff_ms: u64,
}

impl Default for WebhookDispatcher {
    fn default() -> Self {
        Self::new()
    }
}

impl WebhookDispatcher {
    /// Create a new WebhookDispatcher with default configuration.
    pub fn new() -> Self {
        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .unwrap_or_default(),
            max_retries: 3,
            initial_backoff_ms: 100,
        }
    }

    /// Check if a path matches a webhook glob pattern.
    pub fn matches_pattern(pattern: &str, path: &str) -> bool {
        if pattern == "*" || pattern == "**" || pattern == path {
            return true;
        }
        let re_str = crate::paths::glob_to_regex(pattern);
        match regex::Regex::new(&re_str) {
            Ok(re) => re.is_match(path),
            Err(_) => path.starts_with(pattern.trim_end_matches('*')),
        }
    }

    /// Check if an event matches webhook subscribed events list.
    pub fn matches_event(events: &[String], event: &str) -> bool {
        events.iter().any(|e| e == "*" || e == event)
    }

    /// Dispatch an event payload to a single webhook with signature and retry logic.
    pub async fn dispatch_one(
        &self,
        fs: &SurrealFs,
        webhook: &WebhookRecord,
        event: &str,
        path: &str,
        payload: &Value,
    ) -> Result<WebhookDeliveryRecord> {
        let body = serde_json::to_vec(payload)?;
        let sig = compute_signature(&webhook.secret, &body);
        let timestamp = Utc::now().to_rfc3339();

        let mut attempts = 0;
        let mut last_status = None;
        let mut last_err = None;
        let mut success = false;

        let start_time = std::time::Instant::now();

        for attempt in 1..=self.max_retries {
            attempts = attempt;
            let req = self
                .client
                .post(&webhook.url)
                .header("Content-Type", "application/json")
                .header("X-SurrealFS-Event", event)
                .header("X-SurrealFS-Path", path)
                .header("X-SurrealFS-Timestamp", &timestamp)
                .header("X-SurrealFS-Signature", &sig)
                .body(body.clone());

            match req.send().await {
                Ok(resp) => {
                    let status = resp.status().as_u16();
                    last_status = Some(status);
                    if resp.status().is_success() {
                        success = true;
                        last_err = None;
                        break;
                    } else {
                        last_err = Some(format!("HTTP status {}", status));
                    }
                }
                Err(err) => {
                    last_err = Some(err.to_string());
                }
            }

            if attempt < self.max_retries {
                let backoff = self.initial_backoff_ms * (1 << (attempt - 1));
                tokio::time::sleep(Duration::from_millis(backoff)).await;
            }
        }

        let duration_ms = start_time.elapsed().as_millis() as u64;
        debug!(
            "Webhook {} dispatched (event={}, attempts={}, success={}, duration={}ms)",
            webhook.url, event, attempts, success, duration_ms
        );

        let delivery = WebhookDeliveryRecord {
            id: None,
            webhook_id: webhook.id.clone(),
            owner: webhook.owner.clone(),
            event: event.to_string(),
            path: path.to_string(),
            status_code: last_status,
            success,
            error: last_err,
            attempts,
            delivered_at: Some(Utc::now()),
        };

        // Best effort delivery record logging
        if let Err(e) = fs.log_webhook_delivery(&delivery).await {
            warn!("Failed to log webhook delivery: {}", e);
        }

        Ok(delivery)
    }

    /// Dispatch event to all matching active webhooks.
    pub async fn dispatch_event(
        &self,
        fs: &SurrealFs,
        event: &str,
        path: &str,
        payload: &Value,
    ) -> Result<Vec<WebhookDeliveryRecord>> {
        let webhooks = fs.list_webhooks().await?;
        let mut deliveries = Vec::new();

        for hook in webhooks {
            if !hook.active {
                continue;
            }
            if !Self::matches_event(&hook.events, event) {
                continue;
            }
            if !Self::matches_pattern(&hook.path_pattern, path) {
                continue;
            }
            let delivery = self.dispatch_one(fs, &hook, event, path, payload).await?;
            deliveries.push(delivery);
        }

        Ok(deliveries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_signature() {
        let secret = "test-secret";
        let body = b"{\"event\":\"file_created\",\"path\":\"/notes.md\"}";
        let sig = compute_signature(secret, body);
        assert!(sig.starts_with("sha256="));
        assert_eq!(sig.len(), 7 + 64);
    }

    #[test]
    fn test_pattern_matching() {
        assert!(WebhookDispatcher::matches_pattern("*", "/notes.md"));
        assert!(WebhookDispatcher::matches_pattern(
            "**",
            "/docs/api/spec.md"
        ));
        assert!(WebhookDispatcher::matches_pattern(
            "/events/*",
            "/events/item1"
        ));
        assert!(!WebhookDispatcher::matches_pattern(
            "/events/*",
            "/other/item1"
        ));
    }

    #[test]
    fn test_event_matching() {
        let events = vec!["file_created".to_string(), "file_updated".to_string()];
        assert!(WebhookDispatcher::matches_event(&events, "file_created"));
        assert!(!WebhookDispatcher::matches_event(&events, "file_deleted"));

        let wildcard = vec!["*".to_string()];
        assert!(WebhookDispatcher::matches_event(&wildcard, "any_event"));
    }
}
