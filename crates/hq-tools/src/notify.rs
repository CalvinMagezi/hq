//! `notify_owner`: lets HQ, and any agent reaching HQ over MCP, put a notification in the
//! owner's inbox. The web server pushes it to every browser that turned Web Push on
//! (`docs/WEB_PUSH.md`); with `channels: web_and_chat` the value bus also sends it to the chat
//! relay. The tool only writes the inbox row, so it works from any process sharing the vault
//! database, and holds no push keys itself.

use std::sync::Arc;

use anyhow::{Result, bail};
use async_trait::async_trait;
use chrono::{Duration, Utc};
use hq_core::types::{ValueItem, ValueKind};
use hq_db::Database;
use serde_json::{Value, json};

use crate::registry::{HqTool, ToolPolicy};

/// Inbox source for notifications that stay in the web app (and its pushes).
pub const NOTIFY_SOURCE: &str = "agent_notify";
/// Inbox source for notifications the value bus also sends to the chat relay.
pub const NOTIFY_RELAY_SOURCE: &str = "agent_notify_relay";

const MAX_TITLE_CHARS: usize = 120;
const MAX_BODY_CHARS: usize = 2000;
const MAX_LINK_CHARS: usize = 512;
const MAX_DEDUP_CHARS: usize = 200;

pub fn create_notify_tools(
    db: Arc<Database>,
    config: Option<&hq_core::config::HqConfig>,
) -> Vec<Box<dyn HqTool>> {
    let push = config.map(|c| c.web_push.clone()).unwrap_or_default();
    vec![Box::new(NotifyOwnerTool {
        db,
        max_per_hour: push.max_agent_per_hour,
        push_enabled: push.enabled,
    })]
}

pub struct NotifyOwnerTool {
    db: Arc<Database>,
    max_per_hour: u32,
    push_enabled: bool,
}

impl NotifyOwnerTool {
    pub fn new(db: Arc<Database>, max_per_hour: u32, push_enabled: bool) -> Self {
        Self {
            db,
            max_per_hour,
            push_enabled,
        }
    }
}

#[derive(Debug)]
struct Notice {
    title: String,
    body: String,
    link: Option<String>,
    dedup_key: Option<String>,
    relay: bool,
}

fn str_arg<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn parse(args: &Value) -> Result<Notice> {
    let Some(title) = str_arg(args, "title") else {
        bail!("title is required");
    };
    if title.chars().count() > MAX_TITLE_CHARS {
        bail!("title is longer than {MAX_TITLE_CHARS} characters; put the detail in body");
    }
    let body = str_arg(args, "body").unwrap_or_default();
    if body.chars().count() > MAX_BODY_CHARS {
        bail!("body is longer than {MAX_BODY_CHARS} characters; link to a note or task instead");
    }
    let link = str_arg(args, "link").map(check_link).transpose()?;
    let dedup_key = str_arg(args, "dedup_key").map(str::to_string);
    if dedup_key
        .as_ref()
        .is_some_and(|k| k.chars().count() > MAX_DEDUP_CHARS)
    {
        bail!("dedup_key is longer than {MAX_DEDUP_CHARS} characters");
    }
    let relay = match str_arg(args, "channels").unwrap_or("web") {
        "web" => false,
        "web_and_chat" => true,
        other => bail!("channels must be \"web\" or \"web_and_chat\", not {other:?}"),
    };
    Ok(Notice {
        title: title.to_string(),
        body: body.to_string(),
        link,
        dedup_key,
        relay,
    })
}

/// A link must be a path on HQ's own web app. A full URL is refused, so a session steered by
/// text it read cannot put a phishing link on the owner's lock screen.
fn check_link(link: &str) -> Result<String> {
    let same_origin_path = link.starts_with('/')
        && !link.starts_with("//")
        && !link.contains('\\')
        && !link.chars().any(|c| c.is_control() || c.is_whitespace());
    if !same_origin_path {
        bail!("link must be a path in the HQ web app, such as /tasks or /chat?thread=<id>");
    }
    if link.chars().count() > MAX_LINK_CHARS {
        bail!("link is longer than {MAX_LINK_CHARS} characters");
    }
    Ok(link.to_string())
}

#[async_trait]
impl HqTool for NotifyOwnerTool {
    fn name(&self) -> &str {
        "notify_owner"
    }

    fn description(&self) -> &str {
        "Send the owner a notification: it lands in the HQ web inbox and is pushed to every \
         phone or browser that turned on push notifications, even with HQ closed. Use it when \
         something finished, failed or needs the owner, not for progress chatter. Optional \
         link opens a page of the web app (a path like /tasks). channels: \"web\" (default) \
         or \"web_and_chat\" to also send it to the chat relay. Rate-limited per hour."
    }

    fn category(&self) -> &str {
        "notifications"
    }

    fn search_hint(&self) -> Option<&str> {
        Some("push notification, alert the owner, notify phone, ping me when done")
    }

    fn is_destructive(&self) -> bool {
        false
    }

    fn tool_policy(&self) -> ToolPolicy {
        ToolPolicy::Weak
    }

    fn behavioral_prompt(&self) -> Option<&str> {
        Some(
            "notify_owner reaches the owner's phone. Send one when work they are waiting on \
             finishes or fails, or when you need them; never for routine progress. Lead the \
             title with the outcome (\"Deploy finished\", \"Tests failing on main\"). Give a \
             dedup_key naming the event when the same alert could repeat.",
        )
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "title": {
                    "type": "string",
                    "description": "Short headline, at most 120 characters. Lead with the outcome."
                },
                "body": {
                    "type": "string",
                    "description": "Detail, at most 2000 characters. The push shows the first line or two."
                },
                "link": {
                    "type": "string",
                    "description": "Page of the HQ web app to open on tap: a path such as /tasks, /notifications or /chat?thread=<id>. Full URLs are refused."
                },
                "dedup_key": {
                    "type": "string",
                    "description": "Names the event. A second notification with a key that is still in the inbox, or that the owner dismissed, is dropped."
                },
                "channels": {
                    "type": "string",
                    "enum": ["web", "web_and_chat"],
                    "description": "web (default): inbox and push. web_and_chat: also the chat relay (Telegram or Discord)."
                }
            },
            "required": ["title"]
        })
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let notice = parse(&args)?;
        let db = self.db.clone();
        let max = self.max_per_hour as usize;
        let push_enabled = self.push_enabled;
        tokio::task::spawn_blocking(move || {
            if let Some(key) = &notice.dedup_key
                && let Some(existing) =
                    hq_db::value_items::find_active_by_dedup(&db, ValueKind::Fyi, key)?
            {
                return Ok(json!({
                    "sent": false,
                    "deduplicated": true,
                    "id": existing.id,
                    "note": "a notification with this dedup_key is already in the inbox or was dismissed",
                }));
            }
            let since = Utc::now() - Duration::hours(1);
            let recent = hq_db::value_items::count_from_sources_since(
                &db,
                &[NOTIFY_SOURCE, NOTIFY_RELAY_SOURCE],
                &since,
            )?;
            if recent >= max {
                bail!(
                    "notify_owner limit reached: {recent} notifications in the last hour (web_push.max_agent_per_hour is {max}). Wait, or fold this into one summary later."
                );
            }
            let source = if notice.relay {
                NOTIFY_RELAY_SOURCE
            } else {
                NOTIFY_SOURCE
            };
            let mut item = ValueItem::new(source, ValueKind::Fyi, notice.title, notice.body);
            if let Some(link) = notice.link {
                item = item.with_link(link);
            }
            if let Some(key) = notice.dedup_key {
                item = item.with_dedup_key(key);
            }
            hq_db::value_items::emit(&db, &item)?;
            Ok(json!({
                "sent": true,
                "id": item.id,
                "push": if push_enabled { "queued for every subscribed device" } else { "off on this server (web_push.enabled); the notification is in the web inbox" },
                "relay": notice.relay,
            }))
        })
        .await?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(max: u32) -> (NotifyOwnerTool, Arc<Database>) {
        let db = Arc::new(Database::open_memory().unwrap());
        (NotifyOwnerTool::new(db.clone(), max, true), db)
    }

    fn inbox(db: &Database) -> Vec<ValueItem> {
        hq_db::value_items::list_by_state(db, hq_core::types::ValueState::Pending).unwrap()
    }

    #[tokio::test]
    async fn writes_an_inbox_item_with_its_link() {
        let (tool, db) = tool(5);
        let out = tool
            .execute(json!({"title": "Deploy finished", "body": "v1.2 is live", "link": "/tasks"}))
            .await
            .unwrap();
        assert_eq!(out["sent"], true);
        let items = inbox(&db);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].source_task, NOTIFY_SOURCE);
        assert_eq!(items[0].title, "Deploy finished");
        assert_eq!(items[0].link.as_deref(), Some("/tasks"));
    }

    #[tokio::test]
    async fn web_and_chat_uses_the_relayed_source() {
        let (tool, db) = tool(5);
        tool.execute(json!({"title": "t", "channels": "web_and_chat"}))
            .await
            .unwrap();
        assert_eq!(inbox(&db)[0].source_task, NOTIFY_RELAY_SOURCE);
        assert!(
            tool.execute(json!({"title": "t", "channels": "sms"}))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn refuses_links_off_the_web_app() {
        let (tool, db) = tool(5);
        for link in [
            "https://example.com/login",
            "//example.com/x",
            "/\\example.com",
            "javascript:alert(1)",
            "tasks",
            "/a b",
        ] {
            assert!(
                tool.execute(json!({"title": "t", "link": link}))
                    .await
                    .is_err(),
                "{link}"
            );
        }
        assert!(inbox(&db).is_empty());
        assert!(
            tool.execute(json!({"title": "t", "link": "/chat?thread=abc"}))
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn needs_a_title_and_bounds_its_lengths() {
        let (tool, _) = tool(5);
        assert!(tool.execute(json!({"body": "x"})).await.is_err());
        assert!(tool.execute(json!({"title": "  "})).await.is_err());
        let long = "x".repeat(MAX_TITLE_CHARS + 1);
        assert!(tool.execute(json!({"title": long})).await.is_err());
        let long = "x".repeat(MAX_BODY_CHARS + 1);
        assert!(
            tool.execute(json!({"title": "t", "body": long}))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn a_repeated_dedup_key_is_dropped() {
        let (tool, db) = tool(5);
        let args = json!({"title": "CI red on main", "dedup_key": "ci-main-2026-10-12"});
        assert_eq!(tool.execute(args.clone()).await.unwrap()["sent"], true);
        let again = tool.execute(args).await.unwrap();
        assert_eq!(again["sent"], false);
        assert_eq!(again["deduplicated"], true);
        assert_eq!(inbox(&db).len(), 1);
    }

    #[tokio::test]
    async fn stops_at_the_hourly_limit() {
        let (tool, db) = tool(2);
        for i in 0..2 {
            tool.execute(json!({"title": format!("n{i}")}))
                .await
                .unwrap();
        }
        let err = tool
            .execute(json!({"title": "one too many"}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("limit"), "{err}");
        assert_eq!(inbox(&db).len(), 2);
    }
}
