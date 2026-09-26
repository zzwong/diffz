use diffz_adapters::{github::GithubRules, gitlab::GitlabRules};
use diffz_core::{
    domain::*,
    patch::{ParseLimits, parse_patch},
    provider::{OpenRequest, ReviewRules},
    review::*,
};

fn remote(provider: ProviderId, host: &str, owner: &str, name: &str, pr: u64) -> RemoteTarget {
    RemoteTarget {
        provider,
        repository: RepositoryKey {
            host: host.into(),
            id: 1,
            owner: owner.into(),
            name: name.into(),
        },
        account: "me".into(),
        pr,
        target_tip: "a".into(),
        comparison_base: "b".into(),
        head: "c".into(),
        open: true,
        draft: false,
        pending_review: false,
    }
}

#[test]
fn github_remote_reopens_by_canonical_pull_url() {
    let r = remote(
        ProviderId::GITHUB,
        "github.com",
        "rust-lang",
        "cargo",
        17441,
    );
    assert_eq!(
        (&GithubRules as &dyn ReviewRules).reopen(&r),
        OpenRequest::Remote {
            provider: ProviderId::GITHUB,
            address: "https://github.com/rust-lang/cargo/pull/17441".into(),
        }
    );
}

#[test]
fn gitlab_remote_reopens_by_canonical_mr_url_with_nested_groups() {
    let r = remote(
        ProviderId::GITLAB,
        "gitlab.example.com",
        "group/sub",
        "proj",
        42,
    );
    assert_eq!(
        (&GitlabRules as &dyn ReviewRules).reopen(&r),
        OpenRequest::Remote {
            provider: ProviderId::GITLAB,
            address: "https://gitlab.example.com/group/sub/proj/-/merge_requests/42".into(),
        }
    );
}

#[test]
fn line_links_use_each_host_route_and_encode_paths() {
    let github = remote(ProviderId::GITHUB, "example.com", "group", "repo", 1);
    assert_eq!(
        GithubRules.line_url(&github, "dir/a b.rs", "head", 7),
        "https://example.com/group/repo/blob/head/dir/a%20b.rs#L7"
    );
    let gitlab = remote(ProviderId::GITLAB, "example.com", "group/sub", "repo", 1);
    assert_eq!(
        GitlabRules.line_url(&gitlab, "a.rs", "base", 1),
        "https://example.com/group/sub/repo/-/blob/base/a.rs#L1"
    );
}

fn snapshot() -> Snapshot {
    let patch = parse_patch(
        b"diff --git a/a.md b/a.md\n--- a/a.md\n+++ b/a.md\n@@ -42 +42 @@\n-old\n+new\n",
        ParseLimits::default(),
    )
    .unwrap();
    Snapshot::new(
        "test".into(),
        patch,
        Some(RemoteTarget {
            provider: ProviderId::GITHUB,
            repository: RepositoryKey {
                host: "github.com".into(),
                id: 1,
                owner: "o".into(),
                name: "r".into(),
            },
            account: "alice".into(),
            pr: 3,
            target_tip: "a".repeat(40),
            comparison_base: "a".repeat(40),
            head: "b".repeat(40),
            open: true,
            draft: false,
            pending_review: false,
        }),
        vec![],
    )
}
fn draft(s: &Snapshot) -> Draft {
    Draft {
        id: DraftId("d".into()),
        snapshot: s.id.clone(),
        file: s.patch.files[0].id.clone(),
        side: Side::Right,
        start_line: 42,
        line: 42,
        file_level: false,
        body: "Please clarify".into(),
        version: 1,
        saved_version: 1,
        published: false,
    }
}

fn prepare(s: &Snapshot, d: Draft) -> PreparedReview {
    PreparedReview::prepare(
        &GithubRules,
        OperationId("op".into()),
        s,
        vec![d],
        Verdict::Comment,
        "summary".into(),
    )
    .unwrap()
}
fn github_payload(p: &PreparedReview) -> serde_json::Value {
    serde_json::from_str(GithubRules.payload(p).get()).unwrap()
}

#[test]
fn review_uses_original_source_line_and_commit() {
    let s = snapshot();
    let json = github_payload(&prepare(&s, draft(&s)));
    assert_eq!(json["comments"][0]["line"], 42);
    assert_eq!(json["commit_id"], "b".repeat(40));
}

#[test]
fn file_level_draft_github_payload_is_a_subject_file_comment() {
    let s = snapshot();
    let mut d = draft(&s);
    d.start_line = 0;
    d.line = 0;
    d.file_level = true;
    let json = github_payload(&prepare(&s, d));
    let c = &json["comments"][0];
    assert_eq!(c["path"], "a.md");
    assert_eq!(c["body"], "Please clarify");
    assert_eq!(c["subject_type"], "file");
    assert!(c.get("line").is_none());
    assert!(c.get("side").is_none());
    assert!(c.get("start_line").is_none());
}
