//! Webhook outbound dispatcher with HMAC signatures and exponential retry (§23.2).

use crate::errors::{Result, SurrealFsError};
use crate::fs::SurrealFs;
use crate::models::{WebhookDeliveryRecord, WebhookRecord};
use chrono::Utc;
use hmac::{Hmac, Mac};
use reqwest::Client;
use serde_json::Value;
use sha2::Sha256;
use std::net::IpAddr;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Duration;
use tracing::{debug, warn};

type HmacSha256 = Hmac<Sha256>;

/// Validate a webhook target URL against SSRF vectors (non-HTTP schemes, cloud metadata,
/// link-local, RFC1918 private IPs, and loopback unless `SURREALFS_ALLOW_LOCAL_WEBHOOKS=1`).
pub fn validate_webhook_url(url_str: &str) -> Result<()> {
    let parsed = reqwest::Url::parse(url_str)
        .map_err(|e| SurrealFsError::InvalidArguments(format!("Invalid webhook URL: {}", e)))?;

    let scheme = parsed.scheme();
    if scheme != "http" && scheme != "https" {
        return Err(SurrealFsError::InvalidArguments(format!(
            "Unsupported webhook scheme '{}': only http and https are allowed",
            scheme
        )));
    }

    let host = parsed
        .host_str()
        .ok_or_else(|| SurrealFsError::InvalidArguments("Webhook URL missing host".into()))?
        .trim_matches(|c| c == '[' || c == ']')
        .to_lowercase();

    let allow_local = std::env::var("SURREALFS_ALLOW_LOCAL_WEBHOOKS")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);

    if host == "metadata.google.internal" || host == "169.254.169.254" {
        return Err(SurrealFsError::PermissionDenied(
            "Webhook URL targets forbidden cloud metadata service".into(),
        ));
    }

    if !allow_local && (host == "localhost" || host.ends_with(".localhost") || host.ends_with(".internal")) {
        return Err(SurrealFsError::PermissionDenied(format!(
            "Webhook URL host '{}' is restricted (loopback/internal)",
            host
        )));
    }

    if let Ok(ip) = host.parse::<IpAddr>() {
        let is_blocked = match ip {
            IpAddr::V4(v4) => {
                v4.is_link_local()
                    || v4.is_private()
                    || v4.is_unspecified()
                    || v4.is_broadcast()
                    || (!allow_local && v4.is_loopback())
            }
            IpAddr::V6(v6) => {
                v6.is_unspecified()
                    || (v6.segments()[0] & 0xffc0) == 0xfe80 // link-local fe80::/10
                    || (v6.segments()[0] & 0xfe00) == 0xfc00 // unique local fc00::/7
                    || (!allow_local && v6.is_loopback())
            }
        };
        if is_blocked {
            return Err(SurrealFsError::PermissionDenied(format!(
                "Webhook URL IP '{}' is restricted (private/loopback/link-local)",
                ip
            )));
        }
    }

    Ok(())
}

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
        #[cfg(not(target_arch = "wasm32"))]
        let client = Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_default();
        #[cfg(target_arch = "wasm32")]
        let client = Client::builder().build().unwrap_or_default();

        Self {
            client,
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
        validate_webhook_url(&webhook.url)?;
        let body = serde_json::to_vec(payload)?;
        let sig = compute_signature(&webhook.secret, &body);
        let start_ts = Utc::now();
        let timestamp = start_ts.to_rfc3339();

        let mut attempts = 0;
        let mut last_status = None;
        let mut last_err = None;
        let mut success = false;

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
                #[cfg(not(target_arch = "wasm32"))]
                {
                    let backoff = self.initial_backoff_ms * (1 << (attempt - 1));
                    tokio::time::sleep(Duration::from_millis(backoff)).await;
                }
            }
        }

        let duration_ms = (Utc::now() - start_ts).num_milliseconds().max(0) as u64;
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
        let norm_path = crate::paths::normalize_path(path);

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
            // Enforce tenant readability: if hook.owner is not root and the target file exists,
            // verify hook.owner has read permission on the target path before dispatching.
            if hook.owner != "root" {
                let mut res = fs
                    .db
                    .query("LET $id = fn::sfs_resolve($path); RETURN IF $id IS NONE THEN true ELSE fn::sfs_can_read_grp($id.gate, $id.owner, $id.mode, $owner, $id.group) END;")
                    .bind(("path", norm_path.clone()))
                    .bind(("owner", hook.owner.clone()))
                    .await?;
                let readable: Option<bool> = res.take(1usize).unwrap_or(Some(false));
                if !readable.unwrap_or(false) {
                    continue;
                }
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
    fn test_validate_webhook_url_ssrf_protection() {
        assert!(validate_webhook_url("https://example.com/hook").is_ok());
        assert!(validate_webhook_url("file:///etc/passwd").is_err());
        assert!(validate_webhook_url("http://169.254.169.254/latest/meta-data/").is_err());
        assert!(validate_webhook_url("http://10.0.0.1:8080/hook").is_err());
        assert!(validate_webhook_url("http://192.168.1.1/hook").is_err());
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
