//! Stored bytes that must never change. Values are from the desktop build (serde_json
//! preserve_order on); they must hold with the feature off too.
use diffz_adapters::{github::GithubRules, gitlab::GitlabRules};
use diffz_core::domain::*;
use diffz_core::patch::*;
use diffz_core::provider::ReviewRules;
use diffz_core::review::*;

fn rules(provider: &ProviderId) -> &'static dyn ReviewRules {
    if *provider == ProviderId::GITLAB {
        &GitlabRules
    } else {
        &GithubRules
    }
}

const PATCH: &[u8] =
    b"diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,2 +1,2 @@\n fn a() {}\n-fn b() {}\n+fn c() {}\n";

fn target(provider: ProviderId) -> RemoteTarget {
    RemoteTarget {
        provider,
        repository: RepositoryKey {
            host: "example.com".into(),
            id: 7,
            owner: "group/sub".into(),
            name: "proj".into(),
        },
        account: "alice".into(),
        pr: 12,
        target_tip: "a".repeat(40),
        comparison_base: "b".repeat(40),
        head: "c".repeat(40),
        open: true,
        draft: false,
        pending_review: false,
    }
}

fn snapshot(remote: Option<RemoteTarget>) -> Snapshot {
    Snapshot::new(
        "golden".into(),
        parse_patch(PATCH, ParseLimits::default()).unwrap(),
        remote,
        vec![],
    )
}

fn drafts(s: &Snapshot) -> Vec<Draft> {
    let draft = |id: &str, line: u32, file_level: bool| Draft {
        id: DraftId(id.into()),
        snapshot: s.id.clone(),
        file: s.patch.files[0].id.clone(),
        side: Side::Right,
        start_line: line,
        line,
        file_level,
        body: format!("note {id}"),
        version: 2,
        saved_version: 2,
        published: false,
    };
    vec![draft("line", 2, false), draft("file", 0, true)]
}

fn prepared(provider: ProviderId, verdict: Verdict) -> PreparedReview {
    let s = snapshot(Some(target(provider.clone())));
    let d = drafts(&s);
    PreparedReview::prepare(
        rules(&provider),
        OperationId("op-1".into()),
        &s,
        d,
        verdict,
        "Looks good".into(),
    )
    .unwrap()
}

struct Golden {
    provider: ProviderId,
    target: &'static str,
    snapshot: &'static str,
    fingerprint: &'static str,
    payload: &'static str,
    stored: &'static str,
}

const GOLDEN: [Golden; 2] = [
    Golden {
        provider: ProviderId::GITHUB,
        target: r#"{"repository":{"host":"example.com","id":7,"owner":"group/sub","name":"proj"},"account":"alice","pr":12,"target_tip":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","comparison_base":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","head":"cccccccccccccccccccccccccccccccccccccccc","open":true,"draft":false,"pending_review":false}"#,
        snapshot: "10496732008ca98a86a7d8da037e3a6e870132a7c199889b6aa359f633981b09",
        fingerprint: "77999414d767359bcbe2a892a7a3b19175bf61c405dff58a7b86073f8f312d51",
        payload: r#"{"commit_id":"cccccccccccccccccccccccccccccccccccccccc","event":"APPROVE","body":"Looks good","comments":[{"path":"src/a.rs","body":"note line","line":2,"side":"RIGHT"},{"path":"src/a.rs","body":"note file","subject_type":"file"}]}"#,
        stored: r#"{"id":"op-1","snapshot":"10496732008ca98a86a7d8da037e3a6e870132a7c199889b6aa359f633981b09","target":{"repository":{"host":"example.com","id":7,"owner":"group/sub","name":"proj"},"account":"alice","pr":12,"target_tip":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","comparison_base":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","head":"cccccccccccccccccccccccccccccccccccccccc","open":true,"draft":false,"pending_review":false},"verdict":"Approve","summary":"Looks good","comments":[{"draft":"line","version":2,"path":"src/a.rs","body":"note line","side":"Right","start_line":2,"line":2,"file_level":false},{"draft":"file","version":2,"path":"src/a.rs","body":"note file","side":"Right","start_line":0,"line":0,"file_level":true}],"fingerprint":"77999414d767359bcbe2a892a7a3b19175bf61c405dff58a7b86073f8f312d51"}"#,
    },
    Golden {
        provider: ProviderId::GITLAB,
        target: r#"{"provider":"GitLab","repository":{"host":"example.com","id":7,"owner":"group/sub","name":"proj"},"account":"alice","pr":12,"target_tip":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","comparison_base":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","head":"cccccccccccccccccccccccccccccccccccccccc","open":true,"draft":false,"pending_review":false}"#,
        snapshot: "a6222cb117ce23ab6143bdd25d3e090d5d3b977060a9d0e2514eded2e8a2a5b3",
        fingerprint: "74fcd95c12b41bce8cba95a048f4f52a1e8ef5611e5b0aec434047c21d84a474",
        payload: r#"{"head":"cccccccccccccccccccccccccccccccccccccccc","verdict":"Approve","summary":"Looks good","comments":[{"draft":"line","version":2,"path":"src/a.rs","gitlab_position":{"position_type":"text","base_sha":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","start_sha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","head_sha":"cccccccccccccccccccccccccccccccccccccccc","old_path":"src/a.rs","new_path":"src/a.rs","new_line":2},"body":"note line","side":"Right","start_line":2,"line":2,"file_level":false},{"draft":"file","version":2,"path":"src/a.rs","gitlab_position":{"position_type":"file","base_sha":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","start_sha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","head_sha":"cccccccccccccccccccccccccccccccccccccccc","old_path":"src/a.rs","new_path":"src/a.rs"},"body":"note file","side":"Right","start_line":0,"line":0,"file_level":true}]}"#,
        stored: r#"{"id":"op-1","snapshot":"a6222cb117ce23ab6143bdd25d3e090d5d3b977060a9d0e2514eded2e8a2a5b3","target":{"provider":"GitLab","repository":{"host":"example.com","id":7,"owner":"group/sub","name":"proj"},"account":"alice","pr":12,"target_tip":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","comparison_base":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","head":"cccccccccccccccccccccccccccccccccccccccc","open":true,"draft":false,"pending_review":false},"verdict":"Approve","summary":"Looks good","comments":[{"draft":"line","version":2,"path":"src/a.rs","gitlab_position":{"position_type":"text","base_sha":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","start_sha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","head_sha":"cccccccccccccccccccccccccccccccccccccccc","old_path":"src/a.rs","new_path":"src/a.rs","new_line":2},"body":"note line","side":"Right","start_line":2,"line":2,"file_level":false},{"draft":"file","version":2,"path":"src/a.rs","gitlab_position":{"position_type":"file","base_sha":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","start_sha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","head_sha":"cccccccccccccccccccccccccccccccccccccccc","old_path":"src/a.rs","new_path":"src/a.rs"},"body":"note file","side":"Right","start_line":0,"line":0,"file_level":true}],"fingerprint":"74fcd95c12b41bce8cba95a048f4f52a1e8ef5611e5b0aec434047c21d84a474"}"#,
    },
];

#[test]
fn remote_targets_serialize_unchanged() {
    for g in &GOLDEN {
        assert_eq!(
            serde_json::to_string(&target(g.provider.clone())).unwrap(),
            g.target
        );
        let back: RemoteTarget = serde_json::from_str(g.target).unwrap();
        assert_eq!(back, target(g.provider.clone()));
    }
}

#[test]
fn snapshot_identities_are_unchanged() {
    for g in &GOLDEN {
        let s = snapshot(Some(target(g.provider.clone())));
        assert_eq!(s.id.0, g.snapshot, "{:?}", g.provider);
        assert!(s.verify_identity());
    }
    assert_eq!(
        snapshot(None).id.0,
        "da739d0505758c0f829ab48a153b2cb75485b0e07f9442e702dcbc8544622ac7"
    );
}

#[test]
fn prepared_reviews_are_unchanged() {
    for g in &GOLDEN {
        let r = prepared(g.provider.clone(), Verdict::Approve);
        assert_eq!(r.fingerprint, g.fingerprint, "{:?}", g.provider);
        let payload = rules(&g.provider).payload(&r);
        assert_eq!(payload.get(), g.payload, "{:?}", g.provider);
        assert_eq!(
            serde_json::to_string(&r).unwrap(),
            g.stored,
            "{:?}",
            g.provider
        );
    }
}

#[test]
fn stored_outbox_reviews_still_verify() {
    for g in &GOLDEN {
        let r: PreparedReview = serde_json::from_str(g.stored).unwrap();
        assert!(r.verify(rules(&g.provider)), "{:?}", g.provider);
        let other = if g.provider == ProviderId::GITLAB {
            &GithubRules as &dyn ReviewRules
        } else {
            &GitlabRules
        };
        assert!(!r.verify(other), "{:?}", g.provider);
        assert_eq!(serde_json::to_string(&r).unwrap(), g.stored);
    }
}

#[test]
fn gitlab_rules_still_apply() {
    let s = snapshot(Some(target(ProviderId::GITLAB)));
    let op = || OperationId("op".into());
    assert!(
        PreparedReview::prepare(
            &GitlabRules,
            op(),
            &s,
            vec![],
            Verdict::RequestChanges,
            "x".into()
        )
        .is_err()
    );
    assert!(
        PreparedReview::prepare(
            &GitlabRules,
            op(),
            &s,
            vec![],
            Verdict::Comment,
            "/merge".into()
        )
        .is_err()
    );
    let github = snapshot(Some(target(ProviderId::GITHUB)));
    assert!(
        PreparedReview::prepare(
            &GithubRules,
            op(),
            &github,
            vec![],
            Verdict::RequestChanges,
            "/merge".into()
        )
        .is_ok()
    );
}

#[test]
fn unregistered_providers_round_trip() {
    let explicit = GOLDEN[0]
        .target
        .replacen('{', r#"{"provider":"GitHub","#, 1);
    let back: RemoteTarget = serde_json::from_str(&explicit).unwrap();
    assert_eq!(back.provider, ProviderId::GITHUB);

    let gitea = ProviderId::new("Gitea");
    let json = serde_json::to_string(&target(gitea.clone())).unwrap();
    assert!(json.starts_with(r#"{"provider":"Gitea","#));
    let back: RemoteTarget = serde_json::from_str(&json).unwrap();
    assert_eq!(back.provider, gitea);
    let s = snapshot(Some(target(gitea)));
    assert!(s.verify_identity());
    assert_ne!(s.id.0, GOLDEN[0].snapshot);
    assert_ne!(s.id.0, GOLDEN[1].snapshot);
}
