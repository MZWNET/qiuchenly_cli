use super::client::Client;
use anyhow::{Context, Result};

impl Client {
    pub async fn uninstall_rule_subscriptions(&self) -> Result<Vec<serde_json::Value>> {
        let raw: serde_json::Value = self.get("/uninstall-rules/subscriptions/me", &[]).await?;

        Ok(match raw {
            serde_json::Value::Array(a) => a,
            v => vec![v],
        })
    }

    pub async fn compose_uninstall_rules(&self, collection_ids: &[String]) -> Result<String> {
        let composed: serde_json::Value = self
            .post(
                "/uninstall-rules/compose",
                &serde_json::json!({ "collectionIds": collection_ids }),
            )
            .await?;

        composed
            .get("document")
            .and_then(|v| v.as_str())
            .map(String::from)
            .context("合成结果缺少 document")
    }
}
