//! Read-only provider discussion metadata. Opaque IDs stay separate from legacy
//! numeric comments; finding locations do not imply verified inline anchors.
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscussionSummary {
    pub id: String,
    pub trail_id: String,
    pub title: String,
    pub kind: String,
    pub resolved: bool,
    pub message_count: u64,
    pub created_at: String,
    pub updated_at: String,
    pub last_message_at: Option<String>,
    pub last_message_author: Option<String>,
    #[serde(deserialize_with = "nullable")]
    pub participants: Vec<DiscussionParticipant>,
    pub etag: Option<String>,
    pub created_by: Option<String>,
    pub created_by_identity: Value,
    pub resolved_by: Option<String>,
    pub resolved_by_identity: Value,
    pub resolved_at: Option<String>,
    pub review_comment_id: Option<String>,
    /// Original provider finding metadata, including any versioned location.
    /// Not a verified anchor in the displayed diff.
    pub review_comment: Value,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscussionParticipant {
    pub login: String,
    pub identity: Value,
    pub actor_account_id: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscussionReply {
    pub id: String,
    pub author: String,
    pub identity: Value,
    pub created_at: String,
    pub body: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscussionMessage {
    pub id: String,
    pub etag: Option<String>,
    pub author: String,
    pub identity: Value,
    pub created_at: String,
    pub body: String,
    #[serde(deserialize_with = "nullable")]
    pub replies: Vec<DiscussionReply>,
}
/// A bounded list for one pinned Change. Access filtering still applies even
/// when `has_more` is false; this is not proof of global completeness.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscussionList {
    pub items: Vec<DiscussionSummary>,
    pub has_more: bool,
    /// Event watermarks are not page cursors and do not guarantee a transaction.
    pub event_cursors: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscussionDetail {
    pub discussion: DiscussionSummary,
    #[serde(deserialize_with = "nullable")]
    pub messages: Vec<DiscussionMessage>,
    pub event_cursor: String,
}
fn nullable<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> std::result::Result<Vec<T>, D::Error> {
    Ok(Option::<Vec<T>>::deserialize(d)?.unwrap_or_default())
}
