//! Read-only repository discussion summaries and lazy message details.
//! Opaque IDs and provider metadata are preserved, never coerced into GitHub IDs
//! or interpreted as proof of a current inline anchor or reviewer approval.
use super::*;

const MAX_PAGES: usize = 10;
const PAGE_SIZE: usize = 100;
const MAX_MESSAGES: usize = 1000;
const MAX_CONTEXT_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, Deserialize)]
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
#[derive(Debug, Clone, Deserialize)]
pub struct DiscussionParticipant {
    pub login: String,
    pub identity: Value,
    pub actor_account_id: Option<String>,
}
#[derive(Debug, Clone, Deserialize)]
pub struct DiscussionReply {
    pub id: String,
    pub author: String,
    pub identity: Value,
    pub created_at: String,
    pub body: String,
}
#[derive(Debug, Clone, Deserialize)]
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
#[derive(Debug, Clone)]
pub struct DiscussionList {
    pub items: Vec<DiscussionSummary>,
    pub has_more: bool,
    /// Event watermarks are not page cursors and do not guarantee a transaction.
    pub event_cursors: Vec<String>,
}
#[derive(Debug, Clone, Deserialize)]
pub struct DiscussionDetail {
    pub discussion: DiscussionSummary,
    #[serde(deserialize_with = "nullable")]
    pub messages: Vec<DiscussionMessage>,
    pub event_cursor: String,
}
#[derive(Deserialize)]
struct Page {
    #[serde(deserialize_with = "nullable")]
    items: Vec<DiscussionSummary>,
    #[serde(default)]
    next_cursor: Option<String>,
    event_cursor: String,
}
fn nullable<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> std::result::Result<Vec<T>, D::Error> {
    Ok(Option::<Vec<T>>::deserialize(d)?.unwrap_or_default())
}
fn cursor_valid(cursor: &str) -> bool {
    cursor.len() <= 4096 && !cursor.chars().any(char::is_control)
}
fn validate_summary(s: &DiscussionSummary, change: &str) -> Result<()> {
    if !opaque_id(&s.id)
        || s.trail_id != change
        || s.kind.is_empty()
        || s.title.len() > 65536
        || s.review_comment_id
            .as_ref()
            .is_some_and(|id| !opaque_id(id))
    {
        return Err("Entire returned malformed or cross-Change discussion metadata".into());
    }
    Ok(())
}

impl EntireReader {
    /// Read at most 1,000 summaries, without fetching message bodies. Every page
    /// uses processing placement, and both ends revalidate the displayed Change.
    /// Endpoint/shape errors fail explicitly rather than returning an empty list.
    pub fn discussions(
        &self,
        target: &RemoteTarget,
        cancel: Cancellation,
    ) -> Result<DiscussionList> {
        let scope = self.discussion_scope(target, cancel.clone())?;
        let mut result = DiscussionList {
            items: vec![],
            has_more: false,
            event_cursors: vec![],
        };
        let mut cursor = String::new();
        let mut cursors = BTreeSet::new();
        let mut ids = BTreeSet::new();
        let mut bytes = 0;
        for _ in 0..MAX_PAGES {
            if cancel.cancelled() {
                return Err("Entire discussion read cancelled".into());
            }
            let endpoint = query(
                &format!("{}/discussions", scope.endpoint),
                &[("per_page", "100"), ("cursor", &cursor)],
            );
            let raw = self.request_limited(
                &scope.login,
                Some(&scope.origin),
                &endpoint,
                cancel.clone(),
                MAX_CONTEXT_BYTES - bytes,
            )?;
            bytes += raw.len();
            let page: Page = serde_json::from_slice(&raw)
                .map_err(|_| "Entire discussion page has an unsupported shape")?;
            if page.items.len() > PAGE_SIZE || !cursor_valid(&page.event_cursor) {
                return Err("Entire discussion page exceeds its safety limit".into());
            }
            for item in page.items {
                validate_summary(&item, &scope.change)?;
                // A moving list can repeat an ID; reject rather than silently
                // discard potentially changed content and claim completeness.
                if !ids.insert(item.id.clone()) {
                    return Err(
                        "Entire discussion pages repeat an identity; refresh the list".into(),
                    );
                }
                result.items.push(item);
            }
            result.event_cursors.push(page.event_cursor);
            cursor = page.next_cursor.unwrap_or_default();
            result.has_more = !cursor.is_empty();
            if !result.has_more {
                break;
            }
            if !cursor_valid(&cursor) || !cursors.insert(cursor.clone()) {
                return Err(
                    "Entire discussion pagination returned an invalid or repeated cursor".into(),
                );
            }
        }
        self.finish_discussions(&scope, target, cancel)?;
        Ok(result)
    }

    /// Fetch messages only when a caller expands a known opaque discussion ID.
    /// Replies and finding metadata remain separate, unanchored provider data.
    pub fn discussion(
        &self,
        target: &RemoteTarget,
        id: &str,
        cancel: Cancellation,
    ) -> Result<DiscussionDetail> {
        if !opaque_id(id) {
            return Err("Invalid Entire discussion identity".into());
        }
        let scope = self.discussion_scope(target, cancel.clone())?;
        let endpoint = format!("{}/discussions/{}", scope.endpoint, encode(id));
        let raw = self.request_limited(
            &scope.login,
            Some(&scope.origin),
            &endpoint,
            cancel.clone(),
            MAX_CONTEXT_BYTES,
        )?;
        let detail: DiscussionDetail = serde_json::from_slice(&raw)
            .map_err(|_| "Entire discussion detail has an unsupported shape")?;
        validate_summary(&detail.discussion, &scope.change)?;
        if detail.discussion.id != id || !cursor_valid(&detail.event_cursor) {
            return Err("Entire returned a different discussion identity".into());
        }
        let mut ids = BTreeSet::new();
        for message in &detail.messages {
            if !opaque_id(&message.id) || !ids.insert(&message.id) || message.body.len() > 65536 {
                return Err("Entire discussion has invalid or repeated message identities".into());
            }
            for reply in &message.replies {
                if !opaque_id(&reply.id) || !ids.insert(&reply.id) || reply.body.len() > 65536 {
                    return Err("Entire discussion has invalid or repeated reply identities".into());
                }
            }
            if ids.len() > MAX_MESSAGES {
                return Err("Entire discussion exceeds the message safety limit".into());
            }
        }
        self.finish_discussions(&scope, target, cancel)?;
        Ok(detail)
    }

    fn discussion_scope(&self, target: &RemoteTarget, cancel: Cancellation) -> Result<Scope> {
        if target.provider != ProviderId::ENTIRE {
            return Err("Discussion target is not Entire".into());
        }
        let entire = target
            .entire
            .as_ref()
            .ok_or("Entire discussion identity is missing")?;
        let address = TrailAddress::parse(&entire.address)?;
        if address.forge != entire.forge || address.project != target.repository.owner {
            return Err("Entire discussion target has inconsistent routing".into());
        }
        let login = self.login(cancel.clone())?;
        if self.account(&login, cancel.clone())? != target.account {
            return Err("Entire account changed; reopen the Trail".into());
        }
        let repo = self.repository(&login, &address, &target.repository.name, cancel.clone())?;
        if repo.id != entire.repository_id
            || repo.placement != entire.placement_id
            || repo.origin != entire.api_origin
        {
            return Err("Entire repository placement changed; reopen the Trail".into());
        }
        let scope = Scope {
            login,
            origin: repo.origin,
            endpoint: format!("/api/v1/repos/{}/trails/{}", repo.placement, target.pr),
            change: entire.change_id.clone(),
        };
        self.finish_discussions(&scope, target, cancel)?;
        Ok(scope)
    }
    fn finish_discussions(
        &self,
        scope: &Scope,
        target: &RemoteTarget,
        cancel: Cancellation,
    ) -> Result<()> {
        if cancel.cancelled() {
            return Err("Entire discussion read cancelled".into());
        }
        let value = self.json(
            &scope.login,
            Some(&scope.origin),
            &scope.endpoint,
            cancel.clone(),
        )?;
        if value["id"].as_str() != Some(&scope.change)
            || value["number"].as_u64() != Some(target.pr)
            || value["head_sha"].as_str() != Some(&target.head)
            || value["base_sha"].as_str() != Some(&target.target_tip)
            || value["merge_base_sha"].as_str() != Some(&target.comparison_base)
        {
            return Err(
                "Entire Change moved; refresh the snapshot before reading discussions".into(),
            );
        }
        let current = self.login(cancel.clone())?;
        if self.account(&current, cancel)? != target.account {
            return Err("Entire account changed during discussion capture".into());
        }
        Ok(())
    }
}
struct Scope {
    login: Login,
    origin: String,
    endpoint: String,
    change: String,
}
