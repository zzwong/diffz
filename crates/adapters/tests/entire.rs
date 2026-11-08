use diffz_adapters::{
    entire::{EntireProvider, EntireReader, EntireRules, TrailAddress, TrailSelection},
    provider::ReviewProvider,
};
use diffz_core::{
    domain::*,
    provider::{Cancellation, ReviewRules},
    review::{PreparedReview, Verdict},
};

const PROJECT: &str = "https://entire.com/et/acme/-/trails/42";
const CHANGE: &str = "https://entire.com/et/acme/-/trails/42/changes/widget/7";
const LEGACY: &str = "https://entire.io/et/acme/widget/trails/7/title-slug";

#[test]
fn entire_addresses_keep_project_and_change_numbers_distinct() {
    let project = TrailAddress::parse(PROJECT).unwrap();
    assert_eq!(project.project, "acme");
    assert_eq!(project.forge, "et");
    assert_eq!(
        project.selection,
        TrailSelection::Project {
            number: 42,
            change: None
        }
    );
    let change = TrailAddress::parse(CHANGE).unwrap();
    assert_eq!(
        change.selection,
        TrailSelection::Project {
            number: 42,
            change: Some(("widget".into(), 7))
        }
    );
    let legacy = TrailAddress::parse(LEGACY).unwrap();
    assert_eq!(legacy.host, "entire.io");
    assert_eq!(
        legacy.selection,
        TrailSelection::Repository {
            repo: "widget".into(),
            number: 7
        }
    );
    let github = TrailAddress::parse("https://entire.com/gh/Owner/widget/trails/7/").unwrap();
    assert_eq!(github.forge, "gh");
    assert_eq!(github.project, "owner");
    assert_eq!(
        TrailAddress::parse(&format!("  {CHANGE}#discussion  ")).unwrap(),
        change
    );
}

#[test]
fn malformed_or_ambiguous_entire_urls_never_become_remote_commands() {
    for input in [
        "42",
        "acme/widget#7",
        "entire.com/et/acme/widget/trails/7",
        "http://entire.com/et/acme/widget/trails/7",
        "https://me:secret@entire.com/et/acme/widget/trails/7",
        "https:///entire.com/et/acme/widget/trails/7",
        "https:////entire.com/et/acme/widget/trails/7",
        "https://%65ntire.com/et/acme/widget/trails/7",
        "https://entire.com:443/et/acme/widget/trails/7",
        "https://entire.com.evil.example/et/acme/widget/trails/7",
        "https://api.entire.com/et/acme/widget/trails/7",
        "https://entire.com/et/acme/widget/trails/0",
        "https://entire.com/et/acme/widget/trails/-7",
        "https://entire.com/et/acme/widget/trails/18446744073709551616",
        "https://entire.com/et/acme/other/../widget/trails/7",
        "https://entire.com/et/acme/other/%2e%2e/widget/trails/7",
        "https://entire.com/et/acme/%2e/widget/trails/7",
        "https://entire.com/et/acme/other/.%2E/widget/trails/7",
        "https://entire.com/et/acme/widget%2Fother/trails/7",
        "https://entire.com/et/acme/widget/trails/7?token=secret",
        "https://entire.com/et/acme/widget/trails/7/slug/extra",
        "https://entire.com/et/acme/-/trails/42/changes/widget/0",
        "https://entire.com/et/acme/-/trails/42/changes/widget/7/extra",
        "https://entire.com/unknown/acme/widget/trails/7",
        "https://entire.com/et/acme\\widget/trails/7",
    ] {
        assert!(TrailAddress::parse(input).is_err(), "accepted {input}");
    }
}

#[test]
fn entire_provider_has_no_publication_transport() {
    let provider = EntireProvider::new(None);
    assert!(provider.accepts(CHANGE));
    assert!(provider.remote().is_err());
    assert!(
        provider
            .open(CHANGE, Cancellation::default())
            .unwrap_err()
            .to_string()
            .contains("entire login")
    );
    let rules = provider.rules();
    assert_eq!(rules.id(), ProviderId::ENTIRE);
    assert_eq!(rules.write_flag(), "");
    assert!(
        rules
            .read_only_reason()
            .unwrap()
            .contains("Local drafts and exports")
    );
    assert!(!rules.supports(Verdict::Approve));
    assert!(!rules.supports(Verdict::RequestChanges));
}

#[cfg(unix)]
#[path = "support/fake_cli.rs"]
mod fake_cli;

#[cfg(unix)]
fn fixture(scenario: &str) -> (tempfile::TempDir, EntireReader) {
    let dir = tempfile::tempdir().unwrap();
    let entire = dir.path().join("entire");
    let curl = dir.path().join("curl");
    fake_cli::write_executable(&entire, include_str!("support/fake_entire.py"));
    fake_cli::write_executable(&curl, include_str!("support/fake_entire_curl.py"));
    std::fs::write(dir.path().join("scenario"), scenario).unwrap();
    (dir, EntireReader::with_curl(entire, curl))
}

#[cfg(unix)]
fn capture(reader: &EntireReader, address: &str) -> diffz_adapters::Result<Snapshot> {
    reader.snapshot(
        &TrailAddress::parse(address).unwrap(),
        Cancellation::default(),
    )
}

#[test]
#[cfg(unix)]
fn entire_capture_uses_processing_primary_and_frozen_refs_without_writes() {
    let (dir, reader) = fixture("normal");
    let snapshot = capture(&reader, PROJECT).unwrap();
    let target = snapshot.remote.as_ref().unwrap();
    let entire = target.entire.as_ref().unwrap();
    assert_eq!(target.provider, ProviderId::ENTIRE);
    assert_eq!(target.repository.id, 0);
    assert_eq!(target.pr, 7);
    assert_eq!(entire.placement_id, "01JBBBBBBBBBBBBBBBBBBBBBBB");
    assert_eq!(entire.change_id, "01JEEEEEEEEEEEEEEEEEEEEEEE");
    assert_eq!(
        entire.project_trail_id.as_deref(),
        Some("01JDDDDDDDDDDDDDDDDDDDDDDD")
    );
    assert_eq!(entire.api_origin, "https://aws-eu-west-1.api.entire.io");
    assert_eq!(entire.address, CHANGE);
    assert_eq!(target.target_tip, "a".repeat(40));
    assert_eq!(target.comparison_base, "c".repeat(40));
    assert_eq!(target.head, "b".repeat(40));
    assert_eq!(target.account, "account_1");
    assert_eq!(snapshot.patch.files.len(), 5);
    assert_eq!(
        snapshot.patch.files[1].old_path.as_ref().unwrap().display(),
        "old.txt"
    );
    assert_eq!(snapshot.patch.files[1].display_path(), "new.txt");
    assert_eq!(
        snapshot.patch.files[2].content,
        diffz_core::patch::ContentKind::Binary
    );
    assert_eq!(
        snapshot.patch.files[3].kind,
        diffz_core::patch::ChangeKind::Added
    );
    assert_eq!(snapshot.patch.files[4].new_mode.as_deref(), Some("100755"));
    assert_eq!(
        snapshot.overview.description.as_deref(),
        Some("Describe intent")
    );
    assert_eq!(snapshot.overview.author.as_deref(), Some("reviewer"));
    assert!(snapshot.overview.captured_at.is_some());
    assert!(snapshot.warnings.is_empty(), "{:?}", snapshot.warnings);
    assert!(snapshot.verify_identity());
    let calls = std::fs::read_to_string(dir.path().join("calls.jsonl")).unwrap();
    assert!(!calls.contains("signature"));
    let api: Vec<serde_json::Value> = calls
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    assert!(
        api.iter()
            .filter(|v| v["args"][0] == "api")
            .all(|v| v["args"][5] == "GET")
    );
    assert!(
        api.iter()
            .any(|v| v["api_origin"] == "https://aws-us-east-2.api.entire.io")
    );
    assert!(
        api.iter()
            .any(|v| v["api_origin"] == "https://aws-eu-west-1.api.entire.io")
    );
}

#[test]
#[cfg(unix)]
fn project_with_exactly_one_complete_change_reopens_the_selected_change() {
    let (_dir, reader) = fixture("normal");
    let project = capture(&reader, PROJECT).unwrap();
    let child = capture(&reader, CHANGE).unwrap();
    assert_eq!(project.id, child.id);
    assert_eq!(
        EntireRules.reopen_address(project.remote.as_ref().unwrap()),
        CHANGE
    );
    let alias = CHANGE.replace("entire.com", "entire.io");
    assert_eq!(capture(&reader, &alias).unwrap().id, child.id);
    let legacy = capture(&reader, LEGACY).unwrap();
    assert_eq!(legacy.id, child.id);
    assert_eq!(
        EntireRules.reopen_address(legacy.remote.as_ref().unwrap()),
        CHANGE
    );
}

#[test]
#[cfg(unix)]
fn explicit_change_reads_need_only_repository_access_and_a_verified_parent() {
    let (dir, reader) = fixture("repo-only");
    let child = capture(&reader, CHANGE).unwrap();
    assert_eq!(capture(&reader, LEGACY).unwrap().id, child.id);
    let calls = std::fs::read_to_string(dir.path().join("calls.jsonl")).unwrap();
    assert!(!calls.contains("/api/v1/projects/resolve/"));
    assert!(!calls.contains("/api/v1/et/acme/trails/"));
    assert!(
        capture(&reader, PROJECT)
            .unwrap_err()
            .to_string()
            .contains("denied")
    );
}

#[test]
#[cfg(unix)]
fn undiscoverable_parents_preserve_the_repository_entry_point() {
    let (_dir, reader) = fixture("missing-parent");
    let snapshot = capture(&reader, LEGACY).unwrap();
    let target = snapshot.remote.as_ref().unwrap();
    assert!(target.entire.as_ref().unwrap().project_trail_id.is_none());
    assert_eq!(
        EntireRules.reopen_address(target),
        "https://entire.com/et/acme/widget/trails/7"
    );
    assert!(
        capture(&reader, CHANGE)
            .unwrap_err()
            .to_string()
            .contains("repository-scoped URL")
    );
}

#[test]
#[cfg(unix)]
fn reopen_destinations_must_still_be_valid_entire_identities() {
    let (_dir, reader) = fixture("normal");
    let snapshot = capture(&reader, CHANGE).unwrap();
    let mut target = snapshot.remote.unwrap();
    for address in [
        "https://evil.example/",
        "javascript:alert(1)",
        "https:///entire.com/et/acme/widget/trails/7",
    ] {
        target.entire.as_mut().unwrap().address = address.into();
        assert_eq!(EntireRules.reopen_address(&target), "");
    }
    target.entire.as_mut().unwrap().address = LEGACY.into();
    assert_eq!(
        EntireRules.reopen_address(&target),
        "https://entire.com/et/acme/widget/trails/7"
    );
}

#[test]
#[cfg(unix)]
fn ambiguous_or_access_filtered_projects_require_an_explicit_change() {
    for scenario in ["multiple", "partial"] {
        let (_dir, reader) = fixture(scenario);
        let err = capture(&reader, PROJECT).unwrap_err().to_string();
        assert!(err.contains("Open an explicit Change URL"), "{err}");
        assert!(err.contains(CHANGE), "{err}");
        assert!(capture(&reader, CHANGE).is_ok(), "explicit {scenario}");
    }
}

#[test]
#[cfg(unix)]
fn project_partial_flag_uses_the_released_optional_boolean_contract() {
    let (_dir, reader) = fixture("omitted-partial-flag");
    assert!(capture(&reader, PROJECT).is_ok());
    let (_dir, reader) = fixture("malformed-partial-flag");
    assert!(
        capture(&reader, PROJECT)
            .unwrap_err()
            .to_string()
            .contains("access-filtering marker")
    );
}

#[test]
#[cfg(unix)]
fn source_is_exact_including_compact_json_and_no_final_newline() {
    let (dir, reader) = fixture("normal");
    let snapshot = capture(&reader, CHANGE).unwrap();
    let target = snapshot.remote.as_ref().unwrap();
    let bytes = reader.source(target, "data.json", &target.head).unwrap();
    assert_eq!(bytes, br#"{"value":2,"nested":{"x":3}}"#);
    assert_eq!(
        reader
            .source(target, "odd name?#.json", &target.comparison_base)
            .unwrap(),
        br#"{"exact":"special path"}"#
    );
    let calls = std::fs::read_to_string(dir.path().join("curl-calls.jsonl")).unwrap();
    assert!(!calls.contains("signature"));
    assert!(!calls.contains("Bearer"));
    assert!(calls.contains("odd%20name%3F%23.json"));
    assert!(reader.source(target, "data.json", "main").is_err());
    assert!(reader.source(target, "data.json", &"f".repeat(40)).is_err());
    assert!(reader.source(target, "../outside", &target.head).is_err());
}

#[test]
#[cfg(unix)]
fn raw_source_refuses_redirects_and_error_bodies() {
    let (dir, reader) = fixture("normal");
    let snapshot = capture(&reader, CHANGE).unwrap();
    let target = snapshot.remote.as_ref().unwrap();
    for scenario in ["source-redirect", "source-404"] {
        std::fs::write(dir.path().join("scenario"), scenario).unwrap();
        assert!(reader.source(target, "data.json", &target.head).is_err());
    }
}

#[test]
#[cfg(unix)]
fn whole_capture_rejects_identity_races_missing_pins_and_untrusted_placements() {
    for (scenario, fragment) in [
        ("moving", "moved throughout capture"),
        ("moving-base", "moved throughout capture"),
        ("moving-parent", "moved throughout capture"),
        ("missing-pin", "/head_sha"),
        ("unknown-status", "lifecycle"),
        ("repository-switch", "identity or primary placement changed"),
        ("account-switch", "account changed"),
        ("wrong-parent", "parent identity"),
        ("wrong-parent-number", "different project Trail number"),
        ("wrong-parent-path", "parent identity"),
        ("foreign-placement", "trusted HTTPS API"),
        ("primary-missing", "processing primary"),
        ("primary-not-ready", "not ready"),
        ("empty-diff", "empty diff"),
        ("missing-status", "API read failed"),
    ] {
        let (_dir, reader) = fixture(scenario);
        let err = capture(&reader, CHANGE).unwrap_err().to_string();
        assert!(err.contains(fragment), "{scenario}: {err}");
    }
}

#[test]
#[cfg(unix)]
fn bare_project_selection_also_confirms_the_resolved_parent_id() {
    let (_dir, reader) = fixture("wrong-parent-id");
    assert!(
        capture(&reader, PROJECT)
            .unwrap_err()
            .to_string()
            .contains("parent identity")
    );
}

#[test]
#[cfg(unix)]
fn terminal_changes_use_the_frozen_snapshot_even_without_a_branch() {
    let (_dir, reader) = fixture("merged");
    let snapshot = capture(&reader, CHANGE).unwrap();
    let target = snapshot.remote.as_ref().unwrap();
    assert!(target.merged);
    assert!(!target.open);
    assert_eq!(target.target_tip, "a".repeat(40));
    assert_eq!(target.head, "b".repeat(40));
}

#[test]
#[cfg(unix)]
fn directory_accepts_a_forge_qualified_identity_without_guessing_its_provider() {
    let (_dir, reader) = fixture("qualified-directory");
    let snapshot = capture(&reader, CHANGE).unwrap();
    assert_eq!(snapshot.remote.as_ref().unwrap().repository.name, "widget");
}

#[test]
#[cfg(unix)]
fn github_mirrors_keep_their_own_namespace_and_processing_primary() {
    let address = CHANGE.replace("/et/", "/gh/");
    for scenario in ["gh-normal", "gh-qualified", "gh-mixed-parent-case"] {
        let (_dir, reader) = fixture(scenario);
        let snapshot = capture(&reader, &address).unwrap();
        let target = snapshot.remote.as_ref().unwrap();
        assert_eq!(target.provider, ProviderId::ENTIRE);
        assert_eq!(target.entire.as_ref().unwrap().forge, "gh");
        assert_eq!(
            target.entire.as_ref().unwrap().placement_id,
            "01JBBBBBBBBBBBBBBBBBBBBBBB"
        );
        assert_eq!(EntireRules.reopen_address(target), address);
        assert_eq!(
            reader.source(target, "data.json", &target.head).unwrap(),
            br#"{"value":2,"nested":{"x":3}}"#
        );
    }
    for scenario in [
        "gh-conflict",
        "gh-unqualified-missing-provider",
        "gh-mismatched-qualified",
    ] {
        let (_dir, reader) = fixture(scenario);
        let err = capture(&reader, &address).unwrap_err().to_string();
        assert!(err.contains("forge-qualified"), "{scenario}: {err}");
    }
}

#[test]
#[cfg(unix)]
fn a_complete_null_file_listing_and_empty_diff_are_a_valid_empty_change() {
    let (_dir, reader) = fixture("null-files");
    let snapshot = capture(&reader, CHANGE).unwrap();
    assert!(snapshot.patch.files.is_empty());
    assert!(snapshot.warnings.is_empty());
    assert!(snapshot.verify_identity());
}

#[test]
#[cfg(unix)]
fn comparison_limits_and_missing_coverage_remain_visible() {
    for (scenario, fragment) in [
        ("truncated", "truncated"),
        ("large", "size-limited"),
        ("coverage", "coverage disagrees"),
    ] {
        let (_dir, reader) = fixture(scenario);
        let snapshot = capture(&reader, CHANGE).unwrap();
        assert!(
            snapshot.warnings.iter().any(|w| w.contains(fragment)),
            "{:?}",
            snapshot.warnings
        );
    }
}

#[test]
#[cfg(unix)]
fn session_context_is_bounded_optional_and_never_persists_prompt_content() {
    let (dir, reader) = fixture("normal");
    let snapshot = capture(&reader, CHANGE).unwrap();
    assert!(
        snapshot
            .overview
            .notices
            .iter()
            .any(|n| n.contains("Entire session session-one"))
    );
    let serialized = serde_json::to_string(&snapshot).unwrap();
    assert!(!serialized.contains("PRIVATE SESSION PROMPT"));
    assert!(!serialized.contains("PRIVATE DISPLAY NAME"));
    let calls = std::fs::read_to_string(dir.path().join("calls.jsonl")).unwrap();
    assert_eq!(
        calls
            .lines()
            .filter(|line| line.contains("/sessions?"))
            .count(),
        1
    );
    assert!(!calls.contains("/sessions/session-one"));
    for scenario in ["sessions-denied", "sessions-absent"] {
        let (_dir, reader) = fixture(scenario);
        let snapshot = capture(&reader, CHANGE).unwrap();
        assert!(
            snapshot
                .overview
                .notices
                .iter()
                .any(|n| n.contains("session metadata is unavailable")),
            "{:?}",
            snapshot.overview.notices
        );
        assert_eq!(snapshot.patch.files.len(), 5);
    }
    let (_dir, reader) = fixture("sessions-partial");
    let snapshot = capture(&reader, CHANGE).unwrap();
    assert!(
        snapshot
            .overview
            .notices
            .iter()
            .any(|n| n.contains("session metadata is incomplete")),
        "{:?}",
        snapshot.overview.notices
    );
}

#[test]
#[cfg(unix)]
fn cancellation_during_optional_session_context_aborts_capture() {
    let (dir, reader) = fixture("sessions-wait");
    let cancel = Cancellation::default();
    let worker_cancel = cancel.clone();
    let worker = std::thread::spawn(move || {
        reader.snapshot(&TrailAddress::parse(CHANGE).unwrap(), worker_cancel)
    });
    let started = std::time::Instant::now();
    while !dir.path().join("sessions-waiting").exists()
        && started.elapsed() < std::time::Duration::from_secs(10)
    {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    cancel.cancel();
    let result = worker.join().unwrap();
    assert!(
        dir.path().join("sessions-waiting").exists(),
        "fixture never reached optional context"
    );
    assert!(result.unwrap_err().to_string().contains("cancelled"));
}

#[test]
#[cfg(unix)]
fn authentication_errors_cannot_disclose_token_bytes() {
    for scenario in [
        "bad-token",
        "foreign-token",
        "nested-core-audience",
        "unauthorized",
    ] {
        let (dir, reader) = fixture(scenario);
        let err = capture(&reader, CHANGE).unwrap_err().to_string();
        assert!(!err.contains("signature"), "{err}");
        assert!(!err.contains("secret"), "{err}");
        assert!(!err.contains("evil.example"), "{err}");
        let calls = std::fs::read_to_string(dir.path().join("calls.jsonl")).unwrap();
        if scenario != "unauthorized" {
            assert_eq!(calls.lines().count(), 1);
        }
    }
}

#[test]
#[cfg(unix)]
fn cli_core_audience_form_is_supported_within_entire_domains() {
    let (_dir, reader) = fixture("core-audience");
    assert!(capture(&reader, CHANGE).is_ok());
}

#[test]
#[cfg(unix)]
fn cancelling_before_capture_never_starts_a_cli() {
    let (dir, reader) = fixture("normal");
    let cancel = Cancellation::default();
    cancel.cancel();
    assert!(
        reader
            .snapshot(&TrailAddress::parse(CHANGE).unwrap(), cancel)
            .is_err()
    );
    assert!(!dir.path().join("calls.jsonl").exists());
}

#[test]
#[cfg(unix)]
fn entire_opaque_ids_are_stored_and_bound_to_snapshot_identity() {
    let (dir, reader) = fixture("normal");
    let snapshot = capture(&reader, CHANGE).unwrap();
    let bytes = serde_json::to_vec(&snapshot).unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains("signature"));
    let restored: Snapshot = serde_json::from_slice(&bytes).unwrap();
    assert!(restored.verify_identity());
    assert_eq!(restored.remote, snapshot.remote);
    for field in [
        "repository_id",
        "placement_id",
        "change_id",
        "project_trail_id",
    ] {
        let mut value = serde_json::to_value(&snapshot).unwrap();
        value["remote"]["entire"][field] =
            serde_json::Value::String("different_opaque_identity".into());
        let tampered: Snapshot = serde_json::from_value(value).unwrap();
        assert!(!tampered.verify_identity(), "unbound {field}");
    }
    let store = diffz_adapters::store::Store::open(&dir.path().join("db")).unwrap();
    store.put_snapshot(&snapshot).unwrap();
    let draft = Draft {
        id: DraftId("entire-local-note".into()),
        snapshot: snapshot.id.clone(),
        file: snapshot.patch.files[0].id.clone(),
        side: Side::Right,
        start_line: 1,
        line: 1,
        file_level: false,
        body: "Explain this change".into(),
        version: 1,
        saved_version: 0,
        published: false,
    };
    store.save_draft(draft.clone()).unwrap();
    assert_eq!(store.drafts(&snapshot.id).unwrap().len(), 1);
    assert!(
        PreparedReview::prepare(
            &EntireRules,
            OperationId("never-send".into()),
            &snapshot,
            vec![draft],
            Verdict::Comment,
            "Local review".into()
        )
        .is_err()
    );
    let point = SourcePoint {
        snapshot: snapshot.id.clone(),
        file: snapshot.patch.files[0].id.clone(),
        side: Side::Right,
        line: 1,
        byte_column: 0,
    };
    let err =
        diffz_core::source_link::source_link(&snapshot, &point, Some(&EntireRules)).unwrap_err();
    assert!(err.contains("immutable source links"));
}

#[test]
#[cfg(unix)]
fn entire_typed_discovery_does_not_fetch_a_diff_and_opens_an_explicit_choice() {
    for scenario in ["multiple", "partial"] {
        let (dir, reader) = fixture(scenario);
        let choices = reader
            .choices(
                &TrailAddress::parse(PROJECT).unwrap(),
                Cancellation::default(),
            )
            .unwrap();
        assert_eq!(choices.title(), "Project intent");
        assert_eq!(choices.possibly_partial(), scenario == "partial");
        assert_eq!(choices.changes()[0].number(), 7);
        let calls = std::fs::read_to_string(dir.path().join("calls.jsonl")).unwrap();
        assert!(!calls.contains("/diff?"));
        assert!(!calls.contains("/compare?"));
        assert!(!calls.contains("/sessions?"));
        let snapshot = reader
            .snapshot_choice(&choices, 0, Cancellation::default())
            .unwrap();
        let entire = snapshot.remote.as_ref().unwrap().entire.as_ref().unwrap();
        assert_eq!(entire.change_id, "01JEEEEEEEEEEEEEEEEEEEEEEE");
        assert_eq!(entire.address, CHANGE);
        assert!(snapshot.verify_identity());
    }
}

#[test]
#[cfg(unix)]
fn entire_choice_replacement_or_removal_fails_before_fetching_diff() {
    for scenario in [
        "choice-replaced",
        "choice-removed",
        "choice-project-switch",
        "choice-placement-switch",
    ] {
        let (dir, reader) = fixture(scenario);
        let choices = reader
            .choices(
                &TrailAddress::parse(PROJECT).unwrap(),
                Cancellation::default(),
            )
            .unwrap();
        assert!(
            reader
                .snapshot_choice(&choices, 0, Cancellation::default())
                .is_err()
        );
        let calls = std::fs::read_to_string(dir.path().join("calls.jsonl")).unwrap();
        assert!(!calls.contains("/diff?"));
    }
}

#[test]
#[cfg(unix)]
fn entire_choice_rejects_account_switch_and_invalid_index() {
    let (dir, reader) = fixture("normal");
    let choices = reader
        .choices(
            &TrailAddress::parse(PROJECT).unwrap(),
            Cancellation::default(),
        )
        .unwrap();
    let calls = std::fs::read_to_string(dir.path().join("calls.jsonl")).unwrap();
    assert!(
        reader
            .snapshot_choice(&choices, usize::MAX, Cancellation::default())
            .is_err()
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("calls.jsonl")).unwrap(),
        calls
    );
    std::fs::write(dir.path().join("scenario"), "account-switch").unwrap();
    assert!(
        reader
            .snapshot_choice(&choices, 0, Cancellation::default())
            .unwrap_err()
            .to_string()
            .contains("account changed")
    );
    assert!(
        !std::fs::read_to_string(dir.path().join("calls.jsonl"))
            .unwrap()
            .contains("/diff?")
    );
}

#[test]
#[cfg(unix)]
fn entire_choice_rechecks_duplicate_identity_at_capture_end() {
    let (_dir, reader) = fixture("choice-final-duplicate");
    let choices = reader
        .choices(
            &TrailAddress::parse(PROJECT).unwrap(),
            Cancellation::default(),
        )
        .unwrap();
    assert!(
        reader
            .snapshot_choice(&choices, 0, Cancellation::default())
            .unwrap_err()
            .to_string()
            .contains("duplicate")
    );
}

#[test]
#[cfg(unix)]
fn entire_discovery_rejects_explicit_urls_without_running_cli() {
    let (dir, reader) = fixture("normal");
    assert!(
        reader
            .choices(
                &TrailAddress::parse(CHANGE).unwrap(),
                Cancellation::default()
            )
            .is_err()
    );
    assert!(!dir.path().join("calls.jsonl").exists());
}

#[test]
#[cfg(unix)]
fn service_preserves_typed_choices_and_persists_only_the_selected_snapshot() {
    use diffz_adapters::service::Services;
    use diffz_core::provider::{OpenRequest, WorkbenchServices};
    use std::sync::Arc;
    for scenario in ["multiple", "partial"] {
        let (dir, reader) = fixture(scenario);
        let mut services = Services::new(&dir.path().join("state"), false).unwrap();
        services.register(Arc::new(EntireProvider::new(Some(Arc::new(reader)))), false);
        let error = services
            .open(
                OpenRequest::Remote {
                    provider: ProviderId::ENTIRE,
                    address: PROJECT.into(),
                },
                Cancellation::default(),
            )
            .unwrap_err();
        let choices = error.choices.unwrap();
        assert_eq!(choices.title(), "Project intent");
        assert!(choices.entries()[0].label.contains("feature/work"));
        assert_eq!(choices.possibly_partial(), scenario == "partial");
        assert!(services.recent().unwrap().is_empty());
        let calls = std::fs::read_to_string(dir.path().join("calls.jsonl")).unwrap();
        let cancel = Cancellation::default();
        cancel.cancel();
        assert!(choices.open(0, cancel).is_err());
        assert!(choices.open(usize::MAX, Cancellation::default()).is_err());
        assert_eq!(
            calls,
            std::fs::read_to_string(dir.path().join("calls.jsonl")).unwrap()
        );
        let opened = choices.open(0, Cancellation::default()).unwrap();
        assert_eq!(
            opened
                .snapshot
                .remote
                .as_ref()
                .unwrap()
                .entire
                .as_ref()
                .unwrap()
                .address,
            CHANGE
        );
        assert_eq!(
            choices.entries()[0].request,
            OpenRequest::Remote {
                provider: ProviderId::ENTIRE,
                address: CHANGE.into()
            }
        );
        assert_eq!(services.recent().unwrap().len(), 1);
        let resumed = services
            .open(
                OpenRequest::Resume(opened.snapshot.id.clone()),
                Cancellation::default(),
            )
            .unwrap();
        assert_eq!(resumed.snapshot.id, opened.snapshot.id);
        assert!(!services.writes_enabled_for(&ProviderId::ENTIRE));
    }
}

#[test]
#[cfg(unix)]
fn service_auto_opens_complete_single_change_and_rejects_stale_selection() {
    use diffz_adapters::service::Services;
    use diffz_core::provider::{OpenRequest, WorkbenchServices};
    use std::sync::Arc;
    let (dir, reader) = fixture("normal");
    let mut services = Services::new(&dir.path().join("state"), false).unwrap();
    services.register(Arc::new(EntireProvider::new(Some(Arc::new(reader)))), false);
    assert!(
        services
            .open(
                OpenRequest::Remote {
                    provider: ProviderId::ENTIRE,
                    address: PROJECT.into()
                },
                Cancellation::default()
            )
            .is_ok()
    );
    std::fs::write(dir.path().join("scenario"), "multiple").unwrap();
    let choices = services
        .open(
            OpenRequest::Remote {
                provider: ProviderId::ENTIRE,
                address: PROJECT.into(),
            },
            Cancellation::default(),
        )
        .unwrap_err()
        .choices
        .unwrap();
    std::fs::write(dir.path().join("scenario"), "account-switch").unwrap();
    assert!(choices.open(0, Cancellation::default()).is_err());
    assert_eq!(services.recent().unwrap().len(), 1);
}

#[test]
#[cfg(unix)]
fn discussion_lists_preserve_identity_pagination_and_unanchored_finding_context() {
    let (dir, reader) = fixture("normal");
    let snapshot = capture(&reader, CHANGE).unwrap();
    let target = snapshot.remote.as_ref().unwrap();
    let list = reader.discussions(target, Cancellation::default()).unwrap();
    assert!(!list.has_more);
    assert_eq!(list.items.len(), 2);
    assert_eq!(list.event_cursors, ["5", "6"]);
    assert_eq!(list.items[0].id, "discussion_1");
    assert_eq!(list.items[0].etag.as_deref(), Some("summary-etag"));
    assert_eq!(list.items[0].last_message_author.as_deref(), Some("agent"));
    assert_eq!(
        list.items[0].last_message_at.as_deref(),
        Some("2026-11-08T00:01:00Z")
    );
    assert_eq!(list.items[0].participants[0].login, "alice");
    assert_eq!(list.items[0].participants[0].identity["kind"], "user");
    assert_eq!(
        list.items[0].participants[0].actor_account_id.as_deref(),
        Some("account_1")
    );
    assert_eq!(list.items[1].kind, "review_comment");
    assert!(list.items[1].resolved);
    assert_eq!(
        list.items[1].review_comment_id.as_deref(),
        Some("finding_2")
    );
    assert_eq!(
        list.items[1].review_comment["code_version_id"],
        "version_old"
    );
    let detail = reader
        .discussion(target, "discussion_1", Cancellation::default())
        .unwrap();
    assert_eq!(detail.messages[0].id, "message_1");
    assert_eq!(detail.messages[0].etag.as_deref(), Some("message-etag"));
    assert_eq!(detail.messages[0].replies[0].id, "reply_1");
    assert_eq!(detail.messages[0].replies[0].identity["kind"], "agent");
    assert_eq!(detail.messages[0].replies[0].body, "");
    let calls = std::fs::read_to_string(dir.path().join("calls.jsonl")).unwrap();
    assert!(!calls.contains("include_messages"));
    for scenario in [
        "discussion-denied",
        "discussion-malformed",
        "discussion-overflow",
        "discussion-bad-cursor",
        "discussion-cross",
        "discussion-duplicate",
        "discussion-loop",
        "discussion-moving",
        "account-switch",
        "foreign-placement",
    ] {
        std::fs::write(dir.path().join("scenario"), scenario).unwrap();
        assert!(
            reader.discussions(target, Cancellation::default()).is_err(),
            "{scenario}"
        );
    }
}

#[test]
#[cfg(unix)]
fn discussion_limits_empty_results_and_lazy_detail_errors_are_explicit() {
    let (dir, reader) = fixture("normal");
    let snapshot = capture(&reader, CHANGE).unwrap();
    let target = snapshot.remote.as_ref().unwrap();
    for scenario in [
        "discussion-duplicate-message",
        "discussion-missing-body",
        "discussion-large-body",
        "discussion-too-many-messages",
        "discussion-wrong-id",
    ] {
        std::fs::write(dir.path().join("scenario"), scenario).unwrap();
        assert!(
            reader
                .discussion(target, "discussion_1", Cancellation::default())
                .is_err(),
            "{scenario}"
        );
    }
    std::fs::write(dir.path().join("scenario"), "discussion-cap").unwrap();
    let limited = reader.discussions(target, Cancellation::default()).unwrap();
    assert_eq!(limited.items.len(), 10);
    assert!(limited.has_more);
    std::fs::write(dir.path().join("scenario"), "discussion-empty").unwrap();
    let empty = reader.discussions(target, Cancellation::default()).unwrap();
    assert!(empty.items.is_empty());
    assert!(!empty.has_more);
    let cancelled = Cancellation::default();
    cancelled.cancel();
    assert!(reader.discussions(target, cancelled).is_err());
    assert!(
        reader
            .discussion(target, "../bad", Cancellation::default())
            .is_err()
    );
}

#[test]
#[cfg(unix)]
fn discussion_raw_byte_limits_reject_padding_escapes_and_aggregate_pages() {
    let (dir, reader) = fixture("normal");
    let snapshot = capture(&reader, CHANGE).unwrap();
    let target = snapshot.remote.as_ref().unwrap();
    for scenario in [
        "discussion-byte-padding",
        "discussion-byte-escapes",
        "discussion-byte-cumulative",
    ] {
        std::fs::write(dir.path().join("scenario"), scenario).unwrap();
        let error = reader
            .discussions(target, Cancellation::default())
            .unwrap_err()
            .to_string();
        assert!(error.contains("output limit"), "{scenario}: {error}");
    }
    std::fs::write(
        dir.path().join("scenario"),
        "discussion-detail-byte-padding",
    )
    .unwrap();
    let error = reader
        .discussion(target, "discussion_1", Cancellation::default())
        .unwrap_err()
        .to_string();
    assert!(error.contains("output limit"), "{error}");
}
