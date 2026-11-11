//! Bounded session metadata for the captured immutable commit range.
//!
//! Wire contracts: RepoSessionsOutputBody and SessionListItem in Entire's
//! regional OpenAPI (2026-11-05). The reader requests base/head and limit=10;
//! this module never requests transcripts or retains the response object.

use diffz_core::review_details::Overview;
use serde_json::Value;
use std::collections::BTreeSet;

const MAX_ROWS: usize = 10;
const UNAVAILABLE: &str = "Entire session metadata is unavailable for the captured commit range. Recordings may exist but be inaccessible or not indexed, including in a separate checkpoint repository.";
const INCOMPLETE: &str = "Entire session metadata is incomplete; additional sessions may be available. At most 10 summaries are shown.";

/// Add an unavailable state without turning an optional metadata read into a
/// failed code review. Transport errors and credentials are never interpolated.
pub(crate) fn unavailable(overview: &mut Overview) {
    notice(overview, UNAVAILABLE);
}

/// Show only a few identifying fields. In particular, do not retain `prompt`,
/// `displayName` (which may fall back to the prompt), or arbitrary session data.
pub(crate) fn apply(
    overview: &mut Overview,
    response: &Value,
    repository: &str,
    placement: &str,
) {
    let Some(envelope) = Envelope::read(response, repository, placement) else {
        unavailable(overview);
        return;
    };
    if !envelope.access {
        unavailable(overview);
        return;
    }

    let mut partial = !envelope.complete || envelope.more || envelope.sessions.len() > MAX_ROWS;
    let mut identities = BTreeSet::new();
    let mut rows = Vec::with_capacity(MAX_ROWS);
    for value in envelope.sessions.iter().take(MAX_ROWS) {
        let Some((id, row)) = session_row(value) else {
            partial = true;
            continue;
        };
        if !identities.insert(id) {
            partial = true;
            continue;
        }
        rows.push(row);
    }

    if rows.is_empty() {
        if partial {
            notice(
                overview,
                "Entire has not provided complete session metadata for the captured commit range; no usable summaries are available here. This does not establish that no agent sessions exist.",
            );
        } else {
            notice(
                overview,
                "Entire reports no indexed sessions linked to the captured commit range. This does not rule out unrecorded or unindexed work.",
            );
        }
        return;
    }

    notice(
        overview,
        "Entire session summaries linked to the captured commit range (metadata only):",
    );
    overview.notices.extend(rows);
    if partial {
        notice(overview, INCOMPLETE);
    }
}

struct Envelope<'a> {
    sessions: &'a [Value],
    access: bool,
    complete: bool,
    more: bool,
}
impl<'a> Envelope<'a> {
    fn read(value: &'a Value, repository: &str, placement: &str) -> Option<Self> {
        let sessions = match value.get("sessions")? {
            Value::Array(rows) => rows.as_slice(),
            Value::Null => &[],
            _ => return None,
        };
        let more = match value.get("next_cursor")? {
            Value::String(cursor) => !cursor.is_empty(),
            Value::Null => false,
            _ => return None,
        };
        let access = value.get("app_has_access")?.as_bool()?;
        let branch = value.get("session_branch_exists")?.as_bool()?;
        let complete = value.get("history_complete")?.as_bool()?;
        let status = value.get("history_status")?.as_str()?;
        let repo = value.get("repo_full_name")?.as_str()?;
        // Cell responses identify either the bare owner/repository pair or,
        // before its display metadata resolves, the canonical placement ULID.
        // Match before rendering any rows or claiming that a range is empty.
        if repo.is_empty() || (!repo.eq_ignore_ascii_case(repository) && repo != placement) {
            return None;
        }
        let source = value.get("source")?.as_str()?;
        Some(Self {
            sessions,
            access,
            // Unknown history states and missing storage must never become an
            // exhaustive empty result, even if another flag claims completeness.
            complete: complete && status == "complete" && branch && !source.is_empty(),
            more,
        })
    }
}

fn session_row(value: &Value) -> Option<(&str, String)> {
    let raw_id = value.get("sessionId")?.as_str()?;
    let id = display_text(raw_id, 80);
    if id.is_empty() {
        return None;
    }
    let agent = nullable_text(value.get("agent")?)?;
    let model = nullable_text(value.get("model")?)?;
    let started = value.get("startedAt")?.as_str()?;
    let active = value.get("lastActivityAt")?.as_str()?;
    let agent = display_text(agent.unwrap_or(""), 48);
    let model = display_text(model.unwrap_or(""), 64);
    let mut row = format!("Entire session {id}");
    row.push_str(if agent.is_empty() {
        " · agent unknown"
    } else {
        " · "
    });
    row.push_str(&agent);
    if !model.is_empty() {
        row.push_str(" · ");
        row.push_str(&model);
    }
    let active = display_text(active, 40);
    let started = display_text(started, 40);
    if !active.is_empty() {
        row.push_str(" · active ");
        row.push_str(&active);
    } else if !started.is_empty() {
        row.push_str(" · started ");
        row.push_str(&started);
    }
    Some((raw_id, row))
}

fn nullable_text(value: &Value) -> Option<Option<&str>> {
    match value {
        Value::Null => Some(None),
        Value::String(s) => Some(Some(s)),
        _ => None,
    }
}

fn notice(overview: &mut Overview, text: &str) {
    if !overview.notices.iter().any(|existing| existing == text) {
        overview.notices.push(text.into());
    }
}

/// Plain, single-line text with bounded scanning and output. Directional
/// controls and invisible formatting cannot disguise the surrounding labels.
fn display_text(raw: &str, limit: usize) -> String {
    let mut out = String::new();
    let mut length = 0;
    let mut pending_space = false;
    let mut truncated = false;
    for (index, ch) in raw.chars().enumerate() {
        if index >= limit * 4 {
            truncated = true;
            break;
        }
        if matches!(ch, '\u{061c}' | '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{2069}' | '\u{feff}')
        {
            continue;
        }
        if ch.is_control() || ch.is_whitespace() {
            pending_space = !out.is_empty();
            continue;
        }
        let needed = 1 + usize::from(pending_space);
        if length + needed > limit {
            truncated = true;
            break;
        }
        if pending_space {
            out.push(' ');
        }
        out.push(ch);
        length += needed;
        pending_space = false;
    }
    if truncated && !out.is_empty() {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn session(id: &str) -> Value {
        json!({
            "sessionId": id,
            "agent": "Claude Code",
            "model": "example-model",
            "startedAt": "2026-11-05T10:00:00Z",
            "lastActivityAt": "2026-11-05T11:00:00Z",
            "displayName": "SENSITIVE_PROMPT_FALLBACK",
            "prompt": "SENSITIVE_RAW_PROMPT",
            "customName": "SENSITIVE_CUSTOM_NAME",
            "unrecognized": {"transcript": "SENSITIVE_TRANSCRIPT"}
        })
    }

    fn response(sessions: Value) -> Value {
        json!({
            "sessions": sessions,
            "next_cursor": null,
            "repo_full_name": "acme/widget",
            "app_has_access": true,
            "session_branch_exists": true,
            "history_status": "complete",
            "history_complete": true,
            "source": "db"
        })
    }

    const REPOSITORY: &str = "acme/widget";
    const PLACEMENT: &str = "01JBBBBBBBBBBBBBBBBBBBBBBB";

    fn mapped(value: Value) -> Overview {
        let mut overview = Overview::default();
        apply(&mut overview, &value, REPOSITORY, PLACEMENT);
        overview
    }

    #[test]
    fn summarizes_only_selected_metadata_without_prompts_or_transcripts() {
        let mut overview = Overview {
            notices: vec!["Local drafts stay local.".into()],
            ..Default::default()
        };
        apply(
            &mut overview,
            &response(json!([session("session-1")])),
            REPOSITORY,
            PLACEMENT,
        );
        assert_eq!(overview.notices[0], "Local drafts stay local.");
        assert!(overview.notices.iter().any(|n| n == "Entire session session-1 · Claude Code · example-model · active 2026-11-05T11:00:00Z"));
        let persisted = serde_json::to_string(&overview).unwrap();
        assert!(!persisted.contains("SENSITIVE"));
        assert!(!persisted.contains("incomplete"));
    }

    #[test]
    fn repository_identity_accepts_canonical_name_or_exact_placement() {
        for repository in [REPOSITORY, "ACME/Widget", PLACEMENT] {
            let mut page = response(json!([session("session-1")]));
            page["repo_full_name"] = json!(repository);
            let got = mapped(page);
            assert!(
                got.notices
                    .iter()
                    .any(|n| n.starts_with("Entire session session-1")),
                "{repository}"
            );
            assert!(!got.notices.iter().any(|n| n == UNAVAILABLE));
        }
    }

    #[test]
    fn contradictory_repository_identity_is_unavailable_before_rows_or_empty_claims() {
        for repository in [
            "other/widget",
            "acme/other",
            "",
            "acme/widget/extra",
            "et/acme/widget",
            " acme/widget",
            "acme/widget ",
            "01JFFFFFFFFFFFFFFFFFFFFFFF",
            "01jbbbbbbbbbbbbbbbbbbbbbbb",
        ] {
            for sessions in [json!([session("must-not-show")]), json!([])] {
                let mut page = response(sessions);
                page["repo_full_name"] = json!(repository);
                let got = mapped(page);
                assert_eq!(got.notices, [UNAVAILABLE], "{repository}");
                assert!(!serde_json::to_string(&got).unwrap().contains("must-not-show"));
            }
        }
    }

    #[test]
    fn null_and_empty_lists_describe_indexed_data_without_claiming_no_agent_work() {
        for sessions in [Value::Null, json!([])] {
            let got = mapped(response(sessions));
            assert_eq!(got.notices.len(), 1);
            assert!(got.notices[0].contains("no indexed sessions"));
            assert!(got.notices[0].contains("unrecorded or unindexed"));
        }
    }

    #[test]
    fn pagination_and_history_flags_never_imply_complete_coverage() {
        for (field, value) in [
            ("next_cursor", json!("another-page")),
            ("history_complete", json!(false)),
            ("history_status", json!("indexing")),
            ("session_branch_exists", json!(false)),
        ] {
            let mut page = response(json!([session("session-1")]));
            page[field] = value.clone();
            let got = mapped(page);
            assert!(got.notices.iter().any(|n| n == INCOMPLETE), "{field}");

            let mut empty = response(json!([]));
            empty[field] = value;
            let got = mapped(empty);
            assert!(
                !got.notices
                    .iter()
                    .any(|n| n.contains("no indexed sessions")),
                "{field}"
            );
            assert!(got.notices[0].contains("not provided complete session metadata"));
        }
    }

    #[test]
    fn inaccessible_or_malformed_envelopes_are_unavailable() {
        let mut denied = response(json!([session("must-not-show")]));
        denied["app_has_access"] = json!(false);
        assert_eq!(mapped(denied).notices, [UNAVAILABLE]);
        for field in [
            "sessions",
            "next_cursor",
            "repo_full_name",
            "app_has_access",
            "session_branch_exists",
            "history_status",
            "history_complete",
            "source",
        ] {
            let mut missing = response(json!([]));
            missing.as_object_mut().unwrap().remove(field);
            assert_eq!(mapped(missing).notices, [UNAVAILABLE], "{field}");
        }
        let mut wrong_type = response(json!([]));
        wrong_type["next_cursor"] = json!(23);
        assert_eq!(mapped(wrong_type).notices, [UNAVAILABLE]);
    }

    #[test]
    fn caps_rows_and_marks_omitted_duplicate_or_malformed_entries() {
        let sessions: Vec<_> = (0..11).map(|i| session(&format!("session-{i}"))).collect();
        let got = mapped(response(json!(sessions)));
        assert_eq!(
            got.notices
                .iter()
                .filter(|n| n.starts_with("Entire session session-"))
                .count(),
            MAX_ROWS
        );
        assert!(!got.notices.iter().any(|n| n.contains("session-10")));
        assert!(got.notices.iter().any(|n| n == INCOMPLETE));

        let got = mapped(response(
            json!([session("same"), session("same"), {"prompt": "SENSITIVE"}]),
        ));
        assert_eq!(
            got.notices
                .iter()
                .filter(|n| n.starts_with("Entire session same"))
                .count(),
            1
        );
        assert!(got.notices.iter().any(|n| n == INCOMPLETE));
    }

    #[test]
    fn nullable_agent_metadata_is_preserved_as_unknown_and_text_is_bounded() {
        let mut row = session("session-1");
        row["agent"] = Value::Null;
        row["model"] = Value::Null;
        row["lastActivityAt"] = json!("");
        let got = mapped(response(json!([row])));
        assert!(
            got.notices
                .iter()
                .any(|n| n.contains("agent unknown · started 2026-11-05T10:00:00Z"))
        );

        let mut row = session("id\n\u{202e}visible");
        row["agent"] = json!("A\r\n\tB\u{001b}\u{2066}C");
        row["model"] = json!("界".repeat(1_000));
        let got = mapped(response(json!([row])));
        let line = got
            .notices
            .iter()
            .find(|n| n.starts_with("Entire session id"))
            .unwrap();
        assert!(line.contains("id visible · A B C"), "{line}");
        assert!(!line.chars().any(char::is_control));
        assert!(!line.contains(['\u{202e}', '\u{2066}']));
        assert!(line.contains('…'));
        assert!(line.chars().count() < 180);
    }
}
