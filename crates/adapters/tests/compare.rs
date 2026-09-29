use diffz_adapters::{
    github::{GithubReader, GithubRules, GithubTarget},
    gitlab::{GitlabReader, GitlabRules, GitlabTarget},
    store::Store,
};
use diffz_core::{
    domain::*,
    provider::{OpenRequest, ReviewRules},
    review::*,
};

fn refs(base: &str, head: &str, direct: bool) -> CompareRefs {
    CompareRefs {
        base: base.into(),
        head: head.into(),
        direct,
    }
}
fn github_compare(url: &str) -> (String, String, String, CompareRefs) {
    match GithubTarget::parse(url).unwrap() {
        GithubTarget::Compare(a) => (a.host, a.owner, a.repo, a.refs),
        other => panic!("{url} parsed as {other:?}"),
    }
}
fn gitlab_compare(url: &str) -> (String, String, CompareRefs) {
    match GitlabTarget::parse(url).unwrap() {
        GitlabTarget::Compare(a) => (a.host, a.project, a.refs),
        other => panic!("{url} parsed as {other:?}"),
    }
}

#[test]
fn github_compare_urls() {
    let (host, owner, repo, r) = github_compare("https://github.com/o/r/compare/v1.0...v2.0");
    assert_eq!(
        (host.as_str(), owner.as_str(), repo.as_str()),
        ("github.com", "o", "r")
    );
    assert_eq!(r, refs("v1.0", "v2.0", false));
    assert_eq!(r.label(), "v1.0...v2.0");
    let direct = github_compare("https://github.com/o/r/compare/v1.0..v2.0").3;
    assert_eq!(direct, refs("v1.0", "v2.0", true));
    assert_eq!(direct.label(), "v1.0..v2.0");
    // Slashes, percent-encoding, cross-fork heads, and the page's own expand=1.
    assert_eq!(
        github_compare("https://github.com/o/r/compare/release/1.0...feature/x?expand=1").3,
        refs("release/1.0", "feature/x", false)
    );
    assert_eq!(
        github_compare("https://ghe.example.com/o/r/compare/release%2F1.0...fork:topic%2Fa%23b/").3,
        refs("release/1.0", "fork:topic/a#b", false)
    );
    // Pull requests keep parsing as before.
    assert!(matches!(
        GithubTarget::parse("https://github.com/o/r/pull/7/files").unwrap(),
        GithubTarget::Pr(a) if a.number == 7
    ));
    assert!(matches!(
        GithubTarget::parse("o/r#7").unwrap(),
        GithubTarget::Pr(_)
    ));
}

#[test]
fn github_compare_rejections() {
    for bad in [
        "http://github.com/o/r/compare/a...b",
        "https://user:pw@github.com/o/r/compare/a...b",
        "https://github.com:8443/o/r/compare/a...b",
        "https://github.com/o/r/compare/a...b?host=evil",
        "https://github.com/o/r/compare/a",
        "https://github.com/o/r/compare/",
        "https://github.com/o/r/compare/...b",
        "https://github.com/o/r/compare/a...",
        "https://github.com/o/r/compare/a....b",
        "https://github.com/o/r/compare/a...b...c",
        "https://github.com/o/r/compare/a..b..c",
        "https://github.com/o/r/compare/-a...b",
        "https://github.com/o/r/compare/a...b%20c",
        "https://github.com/o/r/compare/a...b%",
        "https://github.com/o/r/compare/a...b%ff",
        "https://github.com/o/r/compare/a...x:y:z",
        "https://github.com/o/r/compare/a.../../other",
        "https://github.com/o/../compare/a...b",
        "https://github.com/o/r/compare/a...b/extra?x=1",
    ] {
        assert!(GithubTarget::parse(bad).is_err(), "{bad}");
    }
}

#[test]
fn gitlab_compare_urls() {
    let (host, project, r) = gitlab_compare("https://gitlab.com/g/sub/p/-/compare/v1...v2");
    assert_eq!((host.as_str(), project.as_str()), ("gitlab.com", "g/sub/p"));
    assert_eq!(r, refs("v1", "v2", false));
    assert_eq!(
        gitlab_compare("https://gitlab.com/g/p/-/compare/v1..v2").2,
        refs("v1", "v2", true)
    );
    assert_eq!(
        gitlab_compare("https://gitlab.com/g/p/-/compare/release%2F1...topic/x").2,
        refs("release/1", "topic/x", false)
    );
    assert_eq!(
        gitlab_compare("https://git.example.com/g/p/-/compare?from=release%2F1&to=v2").2,
        refs("release/1", "v2", false)
    );
    assert_eq!(
        gitlab_compare("https://gitlab.com/g/p/-/compare?from=v1&to=v2&straight=true").2,
        refs("v1", "v2", true)
    );
    assert_eq!(
        gitlab_compare("https://gitlab.com/g/p/-/compare?to=v2&from=v1&straight=false").2,
        refs("v1", "v2", false)
    );
    assert!(matches!(
        GitlabTarget::parse("https://gitlab.com/g/p/-/merge_requests/3").unwrap(),
        GitlabTarget::Mr(a) if a.number == 3
    ));
    assert!(matches!(
        GitlabTarget::parse("g/p!3").unwrap(),
        GitlabTarget::Mr(_)
    ));
}

#[test]
fn gitlab_compare_rejections() {
    for bad in [
        "http://gitlab.com/g/p/-/compare/a...b",
        "https://me:pw@gitlab.com/g/p/-/compare/a...b",
        "https://gitlab.com/g/p/-/compare/a",
        "https://gitlab.com/g/p/-/compare",
        "https://gitlab.com/g/p/-/compare?from=a",
        "https://gitlab.com/g/p/-/compare?to=b",
        "https://gitlab.com/g/p/-/compare?from=a&to=b&straight=maybe",
        "https://gitlab.com/g/p/-/compare?from=a&to=b&from_project_id=2",
        "https://gitlab.com/g/p/-/compare/a...b?straight=true",
        "https://gitlab.com/g/p/-/compare/a....b",
        "https://gitlab.com/g/p/-/compare/a...b%20c",
        "https://gitlab.com/g/p/-/compares/a...b",
        "https://gitlab.com/p/-/compare/a...b",
        "https://gitlab.com/g/../p/-/compare/a...b",
        "https://gitlab.com/g/p/-/compare?from=-a&to=b",
    ] {
        assert!(GitlabTarget::parse(bad).is_err(), "{bad}");
    }
}

#[test]
fn each_provider_only_claims_its_own_compare() {
    assert!(GithubTarget::parse("https://gitlab.com/g/p/-/compare/a...b").is_err());
    assert!(GitlabTarget::parse("https://github.com/o/r/compare/a...b").is_err());
}

#[cfg(unix)]
mod loaded {
    use super::*;
    use diffz_core::provider::Cancellation;
    use std::{os::unix::fs::PermissionsExt, path::Path, sync::Arc};

    const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/compare");
    const GH_BASE: &str = "54437197ee79c20678db433d98616fab7ddff1a5";
    const GH_HEAD: &str = "4aad4edebd9f09247d6c6b6784419a74bb116829";
    const GL_FROM: &str = "7a4c44f50c90e9a14f8a2a6a224636398c473c1c";
    const GL_TO: &str = "8a1c38d69aa0544049422f62bb7609e4fc36aca0";
    const GH_URL: &str = "https://github.com/dtolnay/anyhow/compare/1.0.80...1.0.81";
    const GL_URL: &str =
        "https://gitlab.com/gitlab-org/ruby/gems/gitlab-styles/-/compare/14.0.0...14.1.0";

    fn program(dir: &Path) -> std::path::PathBuf {
        let path = dir.join("fake-cli");
        let script = include_str!("support/fake_compare.py").replace("FIXTURES", FIXTURES);
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        path
    }
    fn calls(dir: &Path) -> Vec<String> {
        std::fs::read_to_string(dir.join("calls.log"))
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect()
    }
    fn github(dir: &Path, url: &str) -> Result<Snapshot, String> {
        let GithubTarget::Compare(a) = GithubTarget::parse(url).unwrap() else {
            panic!("not a compare")
        };
        GithubReader::new(program(dir))
            .compare(&a, Cancellation::default())
            .map_err(|e| e.to_string())
    }
    fn gitlab(dir: &Path, url: &str) -> Result<Snapshot, String> {
        let GitlabTarget::Compare(a) = GitlabTarget::parse(url).unwrap() else {
            panic!("not a compare")
        };
        GitlabReader::new(program(dir))
            .compare(&a, Cancellation::default())
            .map_err(|e| e.to_string())
    }

    #[test]
    fn github_compare_pins_the_resolved_commits() {
        let temp = tempfile::tempdir().unwrap();
        let s = github(temp.path(), GH_URL).unwrap();
        let t = s.remote.as_ref().unwrap();
        assert_eq!(t.provider, ProviderId::GITHUB);
        assert_eq!(t.repository.owner, "dtolnay");
        assert_eq!(t.repository.name, "anyhow");
        assert_eq!(t.repository.id, 212936374);
        assert_eq!((t.target_tip.as_str(), t.head.as_str()), (GH_BASE, GH_HEAD));
        assert_eq!(t.comparison_base, GH_BASE);
        assert_eq!((t.pr, t.pending_review, t.account.as_str()), (0, false, ""));
        assert_eq!(t.compare, Some(refs("1.0.80", "1.0.81", false)));
        assert_eq!(s.title, "dtolnay/anyhow  1.0.80...1.0.81");
        assert_eq!(s.patch.files.len(), 3);
        assert!(s.warnings.is_empty(), "{:?}", s.warnings);
        assert!(s.verify_identity());
        assert!(
            s.overview
                .description
                .as_deref()
                .unwrap()
                .starts_with("- `")
        );
        // Names are resolved once; the diff and the file list are read at the resolved commits.
        let mut calls = calls(temp.path());
        calls.sort();
        let pinned = format!("repos/dtolnay/anyhow/compare/{GH_BASE}...{GH_HEAD}");
        assert_eq!(
            calls,
            [
                "repos/dtolnay/anyhow",
                "repos/dtolnay/anyhow/compare/1.0.80...1.0.81",
                pinned.as_str(),
                pinned.as_str(),
            ]
        );
    }

    #[test]
    fn github_two_dot_and_three_dot_are_distinct_snapshots() {
        let temp = tempfile::tempdir().unwrap();
        let three = github(temp.path(), GH_URL).unwrap();
        let two = github(temp.path(), &GH_URL.replace("...", "..")).unwrap();
        assert!(
            two.remote
                .as_ref()
                .unwrap()
                .compare
                .as_ref()
                .unwrap()
                .direct
        );
        assert_eq!(two.title, "dtolnay/anyhow  1.0.80..1.0.81");
        assert_eq!(two.patch.files.len(), three.patch.files.len());
        assert_ne!(two.id, three.id);
    }

    #[test]
    fn github_direct_compare_needs_the_base_to_be_an_ancestor() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("diverged"), "").unwrap();
        let err = github(temp.path(), &GH_URL.replace("...", "..")).unwrap_err();
        assert!(err.contains("use BASE...HEAD"), "{err}");
        // The default three-dot form is what the API computes, so it still opens.
        let s = github(temp.path(), GH_URL).unwrap();
        assert_eq!(s.remote.unwrap().comparison_base, "0".repeat(40));
    }

    #[test]
    fn github_truncation_is_reported_not_hidden() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("many-commits"), "").unwrap();
        let s = github(temp.path(), GH_URL).unwrap();
        assert_eq!(s.warnings.len(), 1, "{:?}", s.warnings);
        assert!(s.warnings[0].contains("only the newest 3 of this compare's 297 commits"));
        std::fs::remove_file(temp.path().join("many-commits")).unwrap();
        std::fs::write(temp.path().join("no-diff"), "").unwrap();
        let s = github(temp.path(), GH_URL).unwrap();
        assert_eq!(s.patch.files.len(), 3);
        assert!(
            s.warnings[0].contains("refused the unified diff"),
            "{:?}",
            s.warnings
        );
    }

    #[test]
    fn gitlab_compare_pins_the_resolved_commits() {
        let temp = tempfile::tempdir().unwrap();
        let s = gitlab(temp.path(), GL_URL).unwrap();
        let t = s.remote.as_ref().unwrap();
        assert_eq!(t.provider, ProviderId::GITLAB);
        assert_eq!(t.repository.owner, "gitlab-org/ruby/gems");
        assert_eq!(t.repository.name, "gitlab-styles");
        assert_eq!(t.repository.id, 4176070);
        assert_eq!((t.target_tip.as_str(), t.head.as_str()), (GL_FROM, GL_TO));
        assert_eq!(t.comparison_base, GL_FROM);
        assert_eq!((t.pr, t.account.as_str()), (0, ""));
        assert_eq!(t.compare, Some(refs("14.0.0", "14.1.0", false)));
        assert_eq!(
            s.title,
            "gitlab-org/ruby/gems/gitlab-styles  14.0.0...14.1.0"
        );
        assert_eq!(s.patch.files.len(), 11);
        assert!(s.warnings.is_empty(), "{:?}", s.warnings);
        assert!(s.verify_identity());
        assert_eq!(
            s.overview.description.as_deref().unwrap().lines().count(),
            8
        );
        let calls = calls(temp.path());
        assert!(calls.iter().any(|c| c.contains("/repository/merge_base?")));
        assert!(calls.iter().any(|c| c.ends_with(&format!(
            "/repository/compare?from={GL_FROM}&to={GL_TO}&straight=false"
        ))));
    }

    #[test]
    fn gitlab_straight_compare_skips_the_merge_base() {
        let temp = tempfile::tempdir().unwrap();
        let s = gitlab(temp.path(), &GL_URL.replace("...", "..")).unwrap();
        assert!(s.remote.unwrap().compare.unwrap().direct);
        let calls = calls(temp.path());
        assert!(!calls.iter().any(|c| c.contains("merge_base")));
        assert!(calls.iter().any(|c| c.ends_with("&straight=true")));
    }

    #[test]
    fn gitlab_omitted_diffs_are_reported() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("collapsed"), "").unwrap();
        std::fs::write(temp.path().join("timeout"), "").unwrap();
        let s = gitlab(temp.path(), GL_URL).unwrap();
        assert_eq!(s.patch.files.len(), 11);
        assert_eq!(s.warnings.len(), 2, "{:?}", s.warnings);
        assert!(s.warnings[0].contains("timed out"));
        assert!(s.warnings[1].contains("too large or collapsed"));
    }

    #[test]
    fn compares_are_read_only_and_reopen_by_url() {
        let temp = tempfile::tempdir().unwrap();
        for (s, rules, url) in [
            (
                github(temp.path(), GH_URL).unwrap(),
                &GithubRules as &dyn ReviewRules,
                GH_URL,
            ),
            (gitlab(temp.path(), GL_URL).unwrap(), &GitlabRules, GL_URL),
        ] {
            let err = PreparedReview::prepare(
                rules,
                OperationId("op".into()),
                &s,
                vec![],
                Verdict::Comment,
                "Looks fine".into(),
            )
            .unwrap_err();
            assert!(err.to_string().contains("read-only"), "{err}");
            assert_eq!(
                rules.reopen(s.remote.as_ref().unwrap()),
                OpenRequest::Remote {
                    provider: rules.id(),
                    address: url.into()
                }
            );
        }
    }

    #[test]
    fn compares_survive_the_store_and_are_not_pull_requests() {
        let temp = tempfile::tempdir().unwrap();
        let s = github(temp.path(), GH_URL).unwrap();
        let store = Arc::new(Store::open(&temp.path().join("db")).unwrap());
        store.put_snapshot(&s).unwrap();
        let back = store.snapshot(&s.id).unwrap();
        assert_eq!(back.remote, s.remote);
        assert!(back.verify_identity());
        assert_ne!(gitlab(temp.path(), GL_URL).unwrap().id, s.id);
    }
}
