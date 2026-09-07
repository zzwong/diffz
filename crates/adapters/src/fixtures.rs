//! Fixture data compiled into the binary, so tests and demos need no credentials.
pub fn patch(id: &str) -> Option<&'static [u8]> {
    match id {
        "F01" => Some(include_bytes!(
            "../../../fixtures/markdown-prose/change.patch"
        )),
        "F02" => Some(include_bytes!(
            "../../../fixtures/markdown-url/change.patch"
        )),
        "F03" => Some(include_bytes!("../../../fixtures/unicode/change.patch")),
        "F04" => Some(include_bytes!("../../../fixtures/crlf-to-lf/change.patch")),
        "F05" => Some(include_bytes!("../../../fixtures/eof-newline/change.patch")),
        "F06" => Some(include_bytes!(
            "../../../fixtures/markdown-whitespace/change.patch"
        )),
        "F07" => Some(include_bytes!(
            "../../../fixtures/split-asymmetric/change.patch"
        )),
        "F08" => Some(include_bytes!("../../../fixtures/quoted-path/change.patch")),
        "F09" => Some(include_bytes!("../../../fixtures/large-line/change.patch")),
        "F10" => Some(include_bytes!("../../../fixtures/many-lines/change.patch")),
        "F11" => Some(include_bytes!("../../../fixtures/mode-only/change.patch")),
        "F12" => Some(include_bytes!(
            "../../../fixtures/malformed-hunk/change.patch"
        )),
        "F13" => Some(include_bytes!(
            "../../../fixtures/combined-diff/change.patch"
        )),
        "F14" => Some(include_bytes!("../../../fixtures/rename-only/change.patch")),
        "F18" => Some(include_bytes!("../../../fixtures/review-flow/change.patch")),
        _ => None,
    }
}

pub fn decorate(snapshot: &mut diffz_core::domain::Snapshot, id: &str) {
    if id != "F18" {
        return;
    }
    use diffz_core::{domain::*, review_details::*};
    snapshot.overview=Overview {
        description:Some("## Sample offline review walkthrough\n\nA **synthetic** fixture covering line threads, check status, compact rows, and scrolling across file boundaries.\n\n- Open the line 12 thread.\n- Keep scrolling when a file boundary arrives; the view continues past it.\n- Nothing here touches the network or performs a remote operation.".into()),
        captured_at:Some(0),
        author:Some("sample-author".into()),
        decision:Some(ReviewDecision::ChangesRequested),
        checks:vec![Check{name:"Example CI workflow".into(),kind:"Workflow".into(),status:"in_progress".into(),conclusion:None,url:None},Check{name:"Example unit tests".into(),kind:"Check".into(),status:"completed".into(),conclusion:Some("success".into()),url:None},Check{name:"Example Linux build".into(),kind:"Check".into(),status:"in_progress".into(),conclusion:None,url:None},Check{name:"Example formatting".into(),kind:"Check".into(),status:"completed".into(),conclusion:Some("failure".into()),url:None}],
        notices:vec!["Offline UI testing uses these made-up statuses; they do not come from real CI runs.".into()],
        conversation:vec![ConversationComment{id:7001,author:"sample-author".into(),created_at:Some("2026-09-04T09:58:00Z".into()),body:"Two files and a single line thread holding two messages make up the sample.".into()}],
    };
    for (id, author, body) in [
        (
            6001,
            "sample-reviewer",
            "<!-- hidden sample metadata -->\nShould the **original source text** of this line stay preserved here?",
        ),
        (
            6002,
            "sample-author",
            "Yes. A wrapped fragment still maps to its original source coordinates.",
        ),
    ] {
        snapshot.comments.push(ThreadComment {
            id,
            root_id: 6001,
            path: "src/review.rs".into(),
            side: Some(Side::Right),
            line: Some(12),
            start_line: None,
            body: body.into(),
            author: author.into(),
            commit_id: String::new(),
            created_at: Some(format!("2026-09-04T10:{:02}:00Z", id % 60)),
        });
    }
}
