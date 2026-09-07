use diffz_core::{domain::*, provider::OpenRequest};

fn remote(provider: ProviderKind, host: &str, owner: &str, name: &str, pr: u64) -> RemoteTarget {
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
        ProviderKind::GitHub,
        "github.com",
        "rust-lang",
        "cargo",
        17441,
    );
    assert_eq!(
        r.open_request(),
        OpenRequest::GitHub("https://github.com/rust-lang/cargo/pull/17441".into())
    );
}

#[test]
fn gitlab_remote_reopens_by_canonical_mr_url_with_nested_groups() {
    let r = remote(
        ProviderKind::GitLab,
        "gitlab.example.com",
        "group/sub",
        "proj",
        42,
    );
    assert_eq!(
        r.open_request(),
        OpenRequest::GitLab("https://gitlab.example.com/group/sub/proj/-/merge_requests/42".into())
    );
}
