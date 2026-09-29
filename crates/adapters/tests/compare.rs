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
    for q in ["diff=split", "diff=unified&w=1", "expand=1&diff=split&w=1"] {
        let url = format!("https://github.com/o/r/compare/a...b?{q}");
        assert_eq!(github_compare(&url).3, refs("a", "b", false), "{url}");
    }
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
        "https://github.com/o/r/compare/a...b?diff=evil",
        "https://github.com/o/r/compare/a...b?w=0",
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
    // The Compare button redirects here with the source project's id attached.
    for (url, r) in [
        (
            "https://gitlab.com/g/p/-/compare/v1...v2?from_project_id=7",
            refs("v1", "v2", false),
        ),
        (
            "https://gitlab.com/g/p/-/compare/v1..v2?from_project_id=7",
            refs("v1", "v2", true),
        ),
        (
            "https://gitlab.com/g/p/-/compare/v1..v2?straight=true",
            refs("v1", "v2", true),
        ),
        (
            "https://gitlab.com/g/p/-/compare/v1...v2?straight=false",
            refs("v1", "v2", false),
        ),
        (
            "https://gitlab.com/g/p/-/compare?from=v1&to=v2&from_project_id=7",
            refs("v1", "v2", false),
        ),
    ] {
        assert_eq!(gitlab_compare(url).2, r, "{url}");
    }
    match GitlabTarget::parse("https://gitlab.com/g/p/-/compare/v1...v2?from_project_id=7").unwrap()
    {
        GitlabTarget::Compare(a) => assert_eq!(a.from_project_id, Some(7)),
        other => panic!("{other:?}"),
    }
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
        "https://gitlab.com/g/p/-/compare?from=a&to=b&from_project_id=x",
        "https://gitlab.com/g/p/-/compare/a...b?straight=true",
        "https://gitlab.com/g/p/-/compare/a..b?straight=false",
        "https://gitlab.com/g/p/-/compare/a...b?from_project_id=x",
        "https://gitlab.com/g/p/-/compare/a...b?from=c",
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
#[path = "support/fake_cli.rs"]
mod fake_cli;

#[cfg(unix)]
mod loaded {
    use super::fake_cli;
    use super::*;
    use diffz_core::{provider::Cancellation, review_details::Release};
    use std::{path::Path, sync::Arc};

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
        fake_cli::write_executable(&path, &script);
        path
    }
    fn calls(dir: &Path) -> Vec<String> {
        std::fs::read_to_string(dir.join("calls.log"))
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect()
    }
    /// The compare alone, as it first shows.
    fn github_compare_only(dir: &Path, url: &str) -> Result<Snapshot, String> {
        let GithubTarget::Compare(a) = GithubTarget::parse(url).unwrap() else {
            panic!("not a compare")
        };
        GithubReader::new(program(dir))
            .compare(&a, Cancellation::default())
            .map_err(|e| e.to_string())
    }
    /// Adds the releases read after the compare shows.
    fn with_releases(
        mut s: Snapshot,
        read: impl FnOnce(&RemoteTarget) -> diffz_adapters::Result<(Vec<Release>, Vec<String>)>,
    ) -> Result<Snapshot, String> {
        let (releases, warnings) = read(s.remote.as_ref().unwrap()).map_err(|e| e.to_string())?;
        s.overview.releases = releases;
        s.warnings.extend(warnings);
        Ok(s)
    }
    fn github(dir: &Path, url: &str) -> Result<Snapshot, String> {
        let s = github_compare_only(dir, url)?;
        with_releases(s, |t| {
            GithubReader::new(program(dir)).releases(t, Cancellation::default())
        })
    }
    fn gitlab(dir: &Path, url: &str) -> Result<Snapshot, String> {
        let GitlabTarget::Compare(a) = GitlabTarget::parse(url).unwrap() else {
            panic!("not a compare")
        };
        let reader = GitlabReader::new(program(dir));
        let s = reader
            .compare(&a, Cancellation::default())
            .map_err(|e| e.to_string())?;
        with_releases(s, |t| reader.releases(t, Cancellation::default()))
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
                // Releases are read after the compare shows, so its file list is read again.
                pinned.as_str(),
                "repos/dtolnay/anyhow/releases?per_page=100",
                "repos/dtolnay/anyhow/tags?per_page=100&page=1",
            ]
        );
        // The only release in range spans the whole compare, so its numbers are the compare's.
        let [release] = &s.overview.releases[..] else {
            panic!("{:?}", s.overview.releases)
        };
        assert_eq!(release.tag.as_deref(), Some("1.0.81"));
        assert_eq!((release.commit.as_str(), release.commits), (GH_HEAD, 3));
        assert_eq!(release.files.len(), 3);
    }

    const GH_RANGE: &str = "https://github.com/dtolnay/anyhow/compare/1.0.78...1.0.81";
    fn summary(s: &Snapshot) -> Vec<(Option<&str>, u64, usize)> {
        s.overview
            .releases
            .iter()
            .map(|r| (r.tag.as_deref(), r.commits, r.files.len()))
            .collect()
    }

    #[test]
    fn github_releases_split_the_range_at_each_tag() {
        let temp = tempfile::tempdir().unwrap();
        let s = github(temp.path(), GH_RANGE).unwrap();
        assert!(s.warnings.is_empty(), "{:?}", s.warnings);
        // 1.0.78 is the base, and 1.0.82 on are past the head, so neither is in range.
        assert_eq!(
            summary(&s),
            [
                (Some("1.0.79"), 6, 5),
                (Some("1.0.80"), 7, 10),
                (Some("1.0.81"), 3, 3)
            ]
        );
        let first = &s.overview.releases[0];
        assert_eq!(first.commit, "71ab53dd2e89ff816bebaa452ad5a968f4c4105d");
        assert!(first.date.as_deref().unwrap().starts_with("2024-01-02T"));
        assert_eq!(
            first.url.as_deref(),
            Some("https://github.com/dtolnay/anyhow/releases/tag/1.0.79")
        );
        assert!(first.notes.as_deref().is_some_and(|n| !n.is_empty()));
        let lib = first.files.iter().find(|f| f.path == "src/lib.rs").unwrap();
        assert_eq!((lib.additions, lib.deletions), (8, 1));
        let by_path = diffz_core::review_details::releases_by_path(&s.overview.releases);
        assert_eq!(by_path["src/lib.rs"], [0, 1, 2]);
        assert_eq!(by_path["src/wrapper.rs"], [1]);
        // Each step is one compare of its two commits, asked for a single commit per page.
        let steps: Vec<_> = calls(temp.path())
            .into_iter()
            .filter(|c| c.ends_with("?per_page=1"))
            .collect();
        assert_eq!(steps.len(), 3, "{steps:?}");
        assert!(steps.contains(&format!(
            "repos/dtolnay/anyhow/compare/{GH_BASE}...{GH_HEAD}?per_page=1"
        )));
    }

    #[test]
    fn github_long_ranges_page_through_their_commits() {
        let temp = tempfile::tempdir().unwrap();
        let listed = github(temp.path(), GH_RANGE).unwrap();
        std::fs::write(temp.path().join("paged"), "").unwrap();
        std::fs::remove_file(temp.path().join("calls.log")).unwrap();
        let s = github(temp.path(), GH_RANGE).unwrap();
        assert_eq!(s.warnings.len(), 1, "{:?}", s.warnings);
        assert!(s.warnings[0].contains("only the newest 10 of this compare's 16 commits"));
        assert_eq!(summary(&s), summary(&listed));
        assert!(
            calls(temp.path())
                .iter()
                .any(|c| c.ends_with("?per_page=100&page=1"))
        );
    }

    #[test]
    fn tags_off_the_range_are_left_out() {
        let temp = tempfile::tempdir().unwrap();
        // One tag on a commit the range merged but whose first-parent path skips it, one on a blob.
        std::fs::write(temp.path().join("side-tag"), "").unwrap();
        std::fs::write(temp.path().join("blob-tag"), "").unwrap();
        let s = github(temp.path(), GH_RANGE).unwrap();
        assert_eq!(
            s.overview
                .releases
                .iter()
                .map(|r| r.tag.as_deref().unwrap())
                .collect::<Vec<_>>(),
            ["1.0.79", "1.0.80", "1.0.81"]
        );
    }

    #[test]
    fn a_compare_opens_without_reading_releases() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("no-tags"), "").unwrap();
        let s = github_compare_only(temp.path(), GH_RANGE).unwrap();
        assert_eq!(s.patch.files.len(), 13);
        assert!(s.overview.releases.is_empty());
        assert!(s.warnings.is_empty(), "{:?}", s.warnings);
        assert!(
            !calls(temp.path())
                .iter()
                .any(|c| c.contains("/tags") || c.contains("/releases"))
        );
        let err = github(temp.path(), GH_RANGE).unwrap_err();
        assert!(err.contains("502"), "{err}");
    }

    #[test]
    fn gitlab_releases_split_the_range_at_each_tag() {
        let temp = tempfile::tempdir().unwrap();
        let s = gitlab(
            temp.path(),
            "https://gitlab.com/gitlab-org/ruby/gems/gitlab-styles/-/compare/13.1.0...14.1.0",
        )
        .unwrap();
        assert!(s.warnings.is_empty(), "{:?}", s.warnings);
        assert_eq!(
            summary(&s),
            [(Some("14.0.0"), 20, 24), (Some("14.1.0"), 8, 11)]
        );
        let [first, last] = &s.overview.releases[..] else {
            unreachable!()
        };
        assert_eq!(
            (first.commit.as_str(), last.commit.as_str()),
            (GL_FROM, GL_TO)
        );
        assert_eq!(
            last.url.as_deref(),
            Some("https://gitlab.com/gitlab-org/ruby/gems/gitlab-styles/-/releases/14.1.0")
        );
        assert!(last.notes.as_deref().is_some_and(|n| !n.is_empty()));
        assert!(last.date.is_some());
        assert!(last.files.iter().any(|f| f.additions + f.deletions > 0));
        // Tags carry their release notes, so only the step compares are added, beside the
        // compare read again once it shows.
        let calls = calls(temp.path());
        assert!(!calls.iter().any(|c| c.contains("/releases")));
        assert_eq!(
            calls
                .iter()
                .filter(|c| c.contains("/repository/compare?"))
                .count(),
            4
        );
    }

    #[test]
    fn gitlab_steps_keep_the_compares_mode_and_skip_tags_without_a_commit() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("blob-tag"), "").unwrap();
        let s = gitlab(
            temp.path(),
            "https://gitlab.com/gitlab-org/ruby/gems/gitlab-styles/-/compare/13.1.0..14.1.0",
        )
        .unwrap();
        assert_eq!(
            summary(&s),
            [(Some("14.0.0"), 20, 24), (Some("14.1.0"), 8, 11)]
        );
        let calls = calls(temp.path());
        let compares: Vec<_> = calls
            .iter()
            .filter(|c| c.contains("/repository/compare?"))
            .collect();
        assert!(
            compares.len() == 4 && compares.iter().all(|c| c.ends_with("&straight=true")),
            "{compares:?}"
        );
    }

    #[test]
    fn snapshots_saved_before_releases_still_load() {
        let old: diffz_core::review_details::Overview = serde_json::from_str(
            r#"{"description":null,"checks":[],"notices":[],"captured_at":null,"conversation":[]}"#,
        )
        .unwrap();
        assert!(old.releases.is_empty());
        let json = serde_json::to_string(&old).unwrap();
        assert!(!json.contains("releases"), "{json}");
    }

    #[test]
    fn github_head_behind_base_pins_the_merge_base() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("behind"), "").unwrap();
        let s = github(temp.path(), GH_URL).unwrap();
        let t = s.remote.as_ref().unwrap();
        assert_eq!(
            (t.target_tip.as_str(), t.head.as_str()),
            (GH_BASE, "1".repeat(40).as_str())
        );
        assert_eq!(t.comparison_base, t.head);
        assert!(s.warnings.is_empty(), "{:?}", s.warnings);
        assert!(calls(temp.path()).contains(&format!(
            "repos/dtolnay/anyhow/compare/{GH_BASE}...{}",
            "1".repeat(40)
        )));
    }

    #[test]
    fn github_identical_refs_open_an_empty_compare() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("identical"), "").unwrap();
        let s = github(temp.path(), GH_URL).unwrap();
        let t = s.remote.as_ref().unwrap();
        assert_eq!((t.target_tip.as_str(), t.head.as_str()), (GH_BASE, GH_BASE));
        assert_eq!(t.comparison_base, GH_BASE);
        assert_eq!(s.overview.description.as_deref(), Some(""));
    }

    #[test]
    fn github_owner_qualified_refs_are_encoded_in_the_request() {
        let temp = tempfile::tempdir().unwrap();
        let url = GH_URL.replace("1.0.80...1.0.81", "dtolnay:1.0.80...fork:1.0.81");
        github(temp.path(), &url).unwrap();
        assert!(calls(temp.path()).contains(
            &"repos/dtolnay/anyhow/compare/dtolnay%3A1.0.80...fork%3A1.0.81".to_string()
        ));
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
    fn gitlab_from_project_id_must_match_the_project() {
        let temp = tempfile::tempdir().unwrap();
        let same = gitlab(temp.path(), &format!("{GL_URL}?from_project_id=4176070")).unwrap();
        assert_eq!(same.remote.unwrap().repository.id, 4176070);
        let err = gitlab(temp.path(), &format!("{GL_URL}?from_project_id=1")).unwrap_err();
        assert!(
            err.contains("cross-project compares are not supported"),
            "{err}"
        );
    }

    #[test]
    fn gitlab_commits_are_listed_oldest_first() {
        let temp = tempfile::tempdir().unwrap();
        let s = gitlab(temp.path(), GL_URL).unwrap();
        let d = s.overview.description.unwrap();
        assert!(
            d.starts_with("- `a95c9583`") && d.lines().last().unwrap().starts_with("- `8a1c38d6`"),
            "{d}"
        );
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
        assert_eq!(back.overview.releases, s.overview.releases);
        assert!(back.verify_identity());
        assert_ne!(gitlab(temp.path(), GL_URL).unwrap().id, s.id);
    }
}
