use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValueKind {
    ActionNeeded,
    Insight,
    Proposal,
    Fyi,
}

impl ValueKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ValueKind::ActionNeeded => "action_needed",
            ValueKind::Insight => "insight",
            ValueKind::Proposal => "proposal",
            ValueKind::Fyi => "fyi",
        }
    }
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "action_needed" => Some(ValueKind::ActionNeeded),
            "insight" => Some(ValueKind::Insight),
            "proposal" => Some(ValueKind::Proposal),
            "fyi" => Some(ValueKind::Fyi),
            _ => None,
        }
    }
    /// Base ranking score by kind. The ranker may refine this later (SP2).
    pub fn base_score(&self) -> f64 {
        match self {
            ValueKind::ActionNeeded => 0.9,
            ValueKind::Proposal => 0.8,
            ValueKind::Insight => 0.5,
            ValueKind::Fyi => 0.3,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValueState {
    Pending,
    Routed,
    Delivered,
    Engaged,
    Dismissed,
    Expired,
}

impl ValueState {
    pub fn as_str(&self) -> &'static str {
        match self {
            ValueState::Pending => "pending",
            ValueState::Routed => "routed",
            ValueState::Delivered => "delivered",
            ValueState::Engaged => "engaged",
            ValueState::Dismissed => "dismissed",
            ValueState::Expired => "expired",
        }
    }
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "pending" => Some(ValueState::Pending),
            "routed" => Some(ValueState::Routed),
            "delivered" => Some(ValueState::Delivered),
            "engaged" => Some(ValueState::Engaged),
            "dismissed" => Some(ValueState::Dismissed),
            "expired" => Some(ValueState::Expired),
            _ => None,
        }
    }
}

/// A normalized unit of value the system wants to surface. The durable artifact
/// (vault note) is referenced by `artifact_path`; this item is the signal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValueItem {
    pub id: String,
    pub source_task: String,
    pub kind: ValueKind,
    pub title: String,
    pub body: String,
    pub artifact_path: Option<String>,
    pub score: f64,
    pub dedup_key: Option<String>,
    pub state: ValueState,
    pub created_at: DateTime<Utc>,
    pub routed_at: Option<DateTime<Utc>>,
    pub delivered_at: Option<DateTime<Utc>>,
    pub engaged_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub engagement: Option<String>,
}

const DEFAULT_TTL_DAYS: i64 = 7;

impl ValueItem {
    pub fn new(
        source_task: impl Into<String>,
        kind: ValueKind,
        title: impl Into<String>,
        body: impl Into<String>,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            source_task: source_task.into(),
            kind,
            title: title.into(),
            body: body.into(),
            artifact_path: None,
            score: kind.base_score(),
            dedup_key: None,
            state: ValueState::Pending,
            created_at: now,
            routed_at: None,
            delivered_at: None,
            engaged_at: None,
            expires_at: Some(now + Duration::days(DEFAULT_TTL_DAYS)),
            engagement: None,
        }
    }

    pub fn with_dedup_key(mut self, key: impl Into<String>) -> Self {
        self.dedup_key = Some(key.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_roundtrips_through_str() {
        for k in [
            ValueKind::ActionNeeded,
            ValueKind::Insight,
            ValueKind::Proposal,
            ValueKind::Fyi,
        ] {
            assert_eq!(ValueKind::from_str(k.as_str()), Some(k));
        }
        assert_eq!(ValueKind::from_str("nope"), None);
    }

    #[test]
    fn state_roundtrips_through_str() {
        for s in [
            ValueState::Pending,
            ValueState::Routed,
            ValueState::Delivered,
            ValueState::Engaged,
            ValueState::Dismissed,
            ValueState::Expired,
        ] {
            assert_eq!(ValueState::from_str(s.as_str()), Some(s));
        }
    }

    #[test]
    fn new_item_defaults_are_sane() {
        let item = ValueItem::new(
            "email-triage",
            ValueKind::ActionNeeded,
            "Reply needed",
            "body",
        );
        assert_eq!(item.state, ValueState::Pending);
        assert_eq!(item.kind, ValueKind::ActionNeeded);
        assert_eq!(item.source_task, "email-triage");
        assert!((item.score - 0.9).abs() < f64::EPSILON);
        assert!(item.expires_at.is_some());
        assert!(item.artifact_path.is_none());
        assert!(!item.id.is_empty());
    }

    #[test]
    fn builders_set_optional_fields() {
        let item = ValueItem::new("memory-consolidation", ValueKind::Insight, "t", "b")
            .with_dedup_key("k1");
        assert_eq!(item.dedup_key.as_deref(), Some("k1"));
    }
}
