//! Read-only gate and CI context from a Change detail's mergeability snapshot.
//!
//! Wire contracts: TrailMergeabilityResponse, GateResult, TrailChecksJSON and
//! TrailCheckRunJSON in https://docs.entire.com/openapi/regional.json (2026-11-05).
//! The Entire v0.12.2 CLI's trail_mergeability_test.go also contains a live-shaped
//! detail fixture. These rows describe evidence; they never grant an approval or
//! derive a merge verdict from the number of passing checks.

use diffz_core::review_details::{Check, Overview};
use serde::Deserialize;
use serde_json::Value;
use url::Url;

#[derive(Deserialize)]
struct Gate {
    #[serde(default)]
    gate_key: String,
    #[serde(default)]
    gate_type: String,
    blocking: Option<bool>,
    status: Option<String>,
    state: Option<String>,
    outcome: Option<String>,
    head_sha: Option<String>,
    evaluated_at_sha: Option<String>,
    stale_reason: Option<String>,
    rationale: Option<String>,
    not_applicable_reason: Option<String>,
    #[serde(default)]
    value: Value,
}

#[derive(Deserialize)]
struct Run {
    name: String,
    status: String,
    conclusion: Option<String>,
    details_url: Option<String>,
    started_at: Option<String>,
    completed_at: Option<String>,
    app_name: Option<String>,
    source: Option<String>,
}

/// Add only evidence that can be associated with the displayed commit. All data
/// comes from the detail already fetched by the reader; this performs no I/O.
pub(crate) fn apply(overview: &mut Overview, mergeability: &Value, expected_head: &str) {
    if !mergeability.is_object() {
        notice(
            overview,
            "Entire did not provide gate and CI results for this revision.",
        );
        return;
    }
    if !same_revision(mergeability["head_sha"].as_str(), expected_head) {
        notice(
            overview,
            "Entire gate and CI results are not pinned to the displayed revision; their status is unavailable.",
        );
        return;
    }

    match mergeability.get("gates") {
        Some(Value::Array(gates)) => {
            for value in gates {
                match serde_json::from_value::<Gate>(value.clone()) {
                    Ok(gate) => add_gate(overview, gate, expected_head),
                    Err(_) => notice(overview, "Some Entire gate results could not be read."),
                }
            }
        }
        Some(Value::Null) => {}
        _ => notice(overview, "Entire did not provide a valid gate result list."),
    }
    add_runs(overview, &mergeability["checks"]);
}

fn same_revision(actual: Option<&str>, expected: &str) -> bool {
    matches!(expected.len(), 40 | 64)
        && expected.bytes().all(|b| b.is_ascii_hexdigit())
        && actual.is_some_and(|sha| sha.eq_ignore_ascii_case(expected))
}

// Status, conclusion, and a reason why a claimed result cannot be used.
type Verdict = (&'static str, Option<&'static str>, Option<&'static str>);

fn gate_verdict(gate: &Gate, head: &str) -> Verdict {
    let status = gate.status.as_deref().unwrap_or("");
    if !matches!(
        status,
        "passed" | "failed" | "pending" | "error" | "skipped" | "not_applicable"
    ) {
        return ("unavailable", None, Some("its status is unknown"));
    }
    let state = gate.state.as_deref();
    if !matches!(
        state,
        None | Some("evaluated" | "running" | "pending" | "disabled" | "stale" | "not_applicable")
    ) {
        return ("unavailable", None, Some("its evaluation state is unknown"));
    }
    if state == Some("not_applicable") || status == "not_applicable" {
        return ("completed", Some("not_applicable"), None);
    }
    if state == Some("disabled") || status == "skipped" {
        return ("completed", Some("skipped"), None);
    }
    if state == Some("stale")
        || gate
            .stale_reason
            .as_deref()
            .is_some_and(|s| !s.trim().is_empty())
        || gate
            .evaluated_at_sha
            .as_deref()
            .is_some_and(|sha| !same_revision(Some(sha), head))
    {
        return (
            "pending",
            None,
            Some("its result is stale for the displayed revision"),
        );
    }
    if !same_revision(gate.head_sha.as_deref(), head) {
        return (
            "pending",
            None,
            Some("it has no result pinned to the displayed revision"),
        );
    }
    if state == Some("running") {
        return ("in_progress", None, None);
    }
    if status == "pending" || state == Some("pending") {
        return ("pending", None, None);
    }
    if !same_revision(gate.evaluated_at_sha.as_deref(), head) {
        return (
            "unavailable",
            None,
            Some("its evaluation revision is unavailable"),
        );
    }
    if gate.outcome.as_deref().is_some_and(|o| o != status) {
        return (
            "unavailable",
            None,
            Some("its status and evaluation outcome disagree"),
        );
    }
    match status {
        "passed" => ("completed", Some("success"), None),
        "failed" => ("completed", Some("failure"), None),
        "error" => ("completed", Some("error"), None),
        _ => ("unavailable", None, Some("its status is unknown")),
    }
}

fn add_gate(overview: &mut Overview, gate: Gate, head: &str) {
    let mut name = display_text(&gate.gate_key);
    if name.is_empty() {
        name = display_text(&gate.gate_type);
    }
    if name.is_empty() {
        notice(overview, "An Entire gate result did not identify its gate.");
        return;
    }
    if gate.blocking == Some(false) {
        name.push_str(" (non-blocking)");
    }
    let (status, conclusion, problem) = gate_verdict(&gate, head);
    if let Some(problem) = problem {
        notice(overview, &format!("Entire gate {name}: {problem}."));
    }
    if conclusion == Some("not_applicable")
        && gate.not_applicable_reason.as_deref() == Some("no_changes")
    {
        notice(
            overview,
            &format!("Entire gate {name}: no code changes; the gate was not evaluated."),
        );
    }
    if conclusion != Some("success")
        && let Some(rationale) = gate.rationale.as_deref().map(display_text)
        && !rationale.is_empty()
    {
        notice(overview, &format!("Entire gate {name}: {rationale}"));
    }
    overview.checks.push(Check {
        name,
        kind: "Gate".into(),
        status: status.into(),
        conclusion: conclusion.map(str::to_owned),
        url: gate.value["web_url"].as_str().and_then(safe_details_url),
    });
}

fn add_runs(overview: &mut Overview, checks: &Value) {
    let availability = checks["availability"].as_str();
    let available = availability == Some("available");
    match availability {
        Some("available") => {}
        Some("unavailable") => notice(
            overview,
            "Entire CI evidence is unavailable for this revision.",
        ),
        Some("not_applicable") => notice(
            overview,
            "Entire reports no applicable CI evidence for this revision.",
        ),
        _ => notice(
            overview,
            "Entire did not report a recognized CI availability for this revision.",
        ),
    }
    let runs = match checks.get("runs") {
        Some(Value::Array(runs)) => runs.as_slice(),
        Some(Value::Null) => &[],
        _ => {
            notice(overview, "Entire did not provide a valid CI run list.");
            return;
        }
    };
    if available && runs.is_empty() {
        notice(
            overview,
            "Entire reports no external CI runs for this revision.",
        );
    }
    for value in runs {
        let Ok(run) = serde_json::from_value::<Run>(value.clone()) else {
            notice(overview, "Some Entire CI results could not be read.");
            continue;
        };
        // The contract includes missing required checks as queued placeholders
        // even when CI evidence is unavailable. Other rows are not meaningful
        // without availability=available, irrespective of their conclusions.
        let placeholder = run.status == "queued"
            && run.started_at.is_none()
            && run.completed_at.is_none()
            && run.conclusion.is_none();
        if !available && !placeholder {
            continue;
        }
        let mut name = display_text(&run.name);
        if name.is_empty() {
            name = "Unnamed Entire CI check".into();
        }
        if let Some(provider) = run
            .app_name
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .or(run.source.as_deref())
            .map(display_text)
            .filter(|s| !s.is_empty())
        {
            name.push_str(&format!(" ({provider})"));
        }
        let (status, conclusion) = match run.status.as_str() {
            "queued" => ("queued", None),
            "in_progress" => ("in_progress", None),
            "completed"
                if matches!(
                    run.conclusion.as_deref(),
                    Some(
                        "success"
                            | "failure"
                            | "neutral"
                            | "skipped"
                            | "cancelled"
                            | "timed_out"
                            | "action_required"
                            | "startup_failure"
                            | "stale"
                    )
                ) =>
            {
                ("completed", run.conclusion.clone())
            }
            _ => {
                notice(
                    overview,
                    &format!("Entire CI check {name}: its status or conclusion is unavailable."),
                );
                ("unavailable", None)
            }
        };
        overview.checks.push(Check {
            name,
            kind: "Check".into(),
            status: status.into(),
            conclusion,
            url: run.details_url.as_deref().and_then(safe_details_url),
        });
    }
}

fn notice(overview: &mut Overview, text: &str) {
    if !overview.notices.iter().any(|existing| existing == text) {
        overview.notices.push(text.into());
    }
}

fn display_text(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn safe_details_url(raw: &str) -> Option<String> {
    // Url::parse otherwise normalizes some control characters and backslashes.
    // The UI opens this link, so refuse ambiguous input instead of repairing it.
    if raw.chars().any(|c| c.is_control() || c.is_whitespace()) || raw.contains('\\') {
        return None;
    }
    let (scheme, rest) = raw.split_once("://")?;
    let authority = rest.split(['/', '?', '#']).next()?;
    if !scheme.eq_ignore_ascii_case("https") || authority.is_empty() || authority.contains('@') {
        return None;
    }
    if raw.as_bytes().windows(3).any(|w| {
        w[0] == b'%'
            && (w[1] as char)
                .to_digit(16)
                .zip((w[2] as char).to_digit(16))
                .is_some_and(|(hi, lo)| ((hi << 4 | lo) as u8).is_ascii_control())
    }) {
        return None;
    }
    let url = Url::parse(raw).ok()?;
    (url.scheme() == "https"
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none())
    .then(|| url.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const HEAD: &str = "b1205a52b53b2206cf168a62c0c08b4e37e4449f";
    const OTHER: &str = "0123456789012345678901234567890123456789";

    fn gate(status: &str) -> Value {
        json!({
            "gate_key": "checks", "gate_type": "checks", "blocking": true,
            "status": status, "state": "evaluated", "outcome": status,
            "head_sha": HEAD, "evaluated_at_sha": HEAD, "stale_reason": null,
            "value": null, "rationale": null
        })
    }

    fn evidence(gates: Vec<Value>, availability: &str, runs: Vec<Value>) -> Value {
        json!({
            "head_sha": HEAD, "mergeable": false, "gates": gates,
            "checks": {"availability": availability, "runs": runs}
        })
    }

    fn loaded(evidence: &Value) -> Overview {
        let mut overview = Overview::default();
        apply(&mut overview, evidence, HEAD);
        overview
    }

    #[test]
    fn current_detail_maps_gates_and_ci_without_granting_an_approval() {
        // Field names and statuses match Entire's pinned, live-shaped CLI
        // fixture, including disabled gates and incomplete check runs.
        let mut approvals = gate("passed");
        approvals["gate_key"] = json!("approvals");
        let mut failed = gate("failed");
        failed["blocking"] = json!(false);
        failed["rationale"] = json!("Build failed\nSee the CI log");
        failed["value"] = json!({"web_url": "https://buildkite.com/acme/app/builds/17"});
        let skipped = json!({
            "gate_key": "up_to_date", "status": "skipped", "state": "disabled",
            "head_sha": HEAD, "evaluated_at_sha": null
        });
        let got = loaded(&evidence(
            vec![approvals, failed, skipped],
            "available",
            vec![
                json!({"name":"lint", "status":"completed", "conclusion":"success", "app_name":"GitHub Actions", "details_url":"https://ci.example/lint"}),
                json!({"name":"test", "status":"completed", "conclusion":"failure"}),
                json!({"name":"CodeQL", "status":"completed", "conclusion":"neutral"}),
                json!({"name":"next", "status":"queued", "conclusion":null}),
                json!({"name":"running", "status":"in_progress", "conclusion":null}),
            ],
        ));
        let statuses: Vec<_> = got
            .checks
            .iter()
            .map(|c| (c.kind.as_str(), c.status.as_str(), c.conclusion.as_deref()))
            .collect();
        assert_eq!(
            statuses,
            vec![
                ("Gate", "completed", Some("success")),
                ("Gate", "completed", Some("failure")),
                ("Gate", "completed", Some("skipped")),
                ("Check", "completed", Some("success")),
                ("Check", "completed", Some("failure")),
                ("Check", "completed", Some("neutral")),
                ("Check", "queued", None),
                ("Check", "in_progress", None),
            ]
        );
        assert_eq!(got.checks[1].name, "checks (non-blocking)");
        assert_eq!(
            got.checks[1].url.as_deref(),
            Some("https://buildkite.com/acme/app/builds/17")
        );
        assert_eq!(got.checks[3].name, "lint (GitHub Actions)");
        assert!(
            got.notices
                .iter()
                .any(|n| n.contains("Build failed See the CI log"))
        );
        assert_eq!(got.decision, None);
    }

    #[test]
    fn absent_or_wrong_parent_revision_never_exposes_completed_results() {
        let good = evidence(
            vec![gate("passed")],
            "available",
            vec![json!({"name":"CI", "status":"completed", "conclusion":"success"})],
        );
        for bad_head in [Value::Null, json!(OTHER), json!("HEAD"), json!(true)] {
            let mut data = good.clone();
            data["head_sha"] = bad_head;
            let got = loaded(&data);
            assert!(got.checks.is_empty());
            assert!(!got.notices.is_empty());
        }
        for data in [Value::Null, json!({}), json!([])] {
            let got = loaded(&data);
            assert!(got.checks.is_empty());
            assert!(!got.notices.is_empty());
        }
        let mut got = Overview::default();
        let mut unpinned = good;
        unpinned["head_sha"] = json!("HEAD");
        apply(&mut got, &unpinned, "HEAD");
        assert!(got.checks.is_empty());
    }

    #[test]
    fn stale_and_unverifiable_gate_evidence_does_not_turn_green() {
        let changes = [
            ("head_sha", json!(OTHER)),
            ("head_sha", Value::Null),
            ("evaluated_at_sha", json!(OTHER)),
            ("evaluated_at_sha", Value::Null),
            ("stale_reason", json!("new_push")),
            ("state", json!("stale")),
            ("state", json!("future_state")),
            ("outcome", json!("failed")),
            ("status", json!("future_status")),
            ("status", Value::Null),
        ];
        for (field, value) in changes {
            let mut stale = gate("passed");
            stale[field] = value;
            let got = loaded(&evidence(vec![stale, gate("passed")], "available", vec![]));
            assert_ne!(
                got.checks[0].conclusion.as_deref(),
                Some("success"),
                "{field}"
            );
            assert_ne!(got.checks[0].status, "completed", "{field}");
            assert_eq!(got.checks[1].conclusion.as_deref(), Some("success"));
            assert!(
                got.notices.iter().any(|n| n.starts_with("Entire gate")),
                "{field}"
            );
        }
    }

    #[test]
    fn in_flight_gate_state_overrides_an_older_passed_outcome() {
        for (state, expected_status) in [("running", "in_progress"), ("pending", "pending")] {
            let mut pending = gate("passed");
            pending["state"] = json!(state);
            let got = loaded(&evidence(vec![pending], "available", vec![]));
            assert_eq!(got.checks[0].status, expected_status);
            assert_eq!(got.checks[0].conclusion, None);
        }
        for status in ["not_applicable", "skipped"] {
            let not_applicable = json!({
                "gate_key":"findings", "status":status, "state":"not_applicable",
                "not_applicable_reason":"no_changes", "head_sha":null, "evaluated_at_sha":null
            });
            let got = loaded(&evidence(vec![not_applicable], "available", vec![]));
            assert_eq!(got.checks[0].conclusion.as_deref(), Some("not_applicable"));
            assert!(got.notices.iter().any(|n| n.contains("was not evaluated")));
        }
    }

    #[test]
    fn unavailable_ci_keeps_only_missing_report_placeholders() {
        for availability in ["unavailable", "not_applicable", "future_availability", ""] {
            let got = loaded(&evidence(
                vec![],
                availability,
                vec![
                    json!({"name":"old success", "status":"completed", "conclusion":"success"}),
                    json!({"name":"required check", "status":"queued", "conclusion":null, "started_at":null}),
                    json!({"name":"already started", "status":"queued", "started_at":"2026-11-05T20:00:00Z"}),
                    json!({"name":"running", "status":"in_progress", "conclusion":null}),
                ],
            ));
            assert_eq!(got.checks.len(), 1, "{availability}");
            assert_eq!(got.checks[0].name, "required check");
            assert_eq!(got.checks[0].status, "queued");
            assert_eq!(got.checks[0].conclusion, None);
            assert!(!got.notices.is_empty());
        }
    }

    #[test]
    fn ci_success_requires_a_complete_run_with_a_known_conclusion() {
        let cases = [
            ("completed", Some("success"), "completed", Some("success")),
            ("completed", Some("failure"), "completed", Some("failure")),
            (
                "completed",
                Some("timed_out"),
                "completed",
                Some("timed_out"),
            ),
            ("completed", Some("neutral"), "completed", Some("neutral")),
            ("completed", Some("skipped"), "completed", Some("skipped")),
            (
                "completed",
                Some("cancelled"),
                "completed",
                Some("cancelled"),
            ),
            (
                "completed",
                Some("action_required"),
                "completed",
                Some("action_required"),
            ),
            (
                "completed",
                Some("startup_failure"),
                "completed",
                Some("startup_failure"),
            ),
            ("completed", Some("stale"), "completed", Some("stale")),
            ("completed", None, "unavailable", None),
            ("completed", Some("future_conclusion"), "unavailable", None),
            ("future_status", Some("success"), "unavailable", None),
            ("queued", Some("success"), "queued", None),
            ("in_progress", Some("success"), "in_progress", None),
        ];
        for (status, conclusion, expected_status, expected_conclusion) in cases {
            let got = loaded(&evidence(
                vec![],
                "available",
                vec![json!({"name":"CI", "status":status, "conclusion":conclusion})],
            ));
            assert_eq!(
                got.checks[0].status, expected_status,
                "{status} {conclusion:?}"
            );
            assert_eq!(
                got.checks[0].conclusion.as_deref(),
                expected_conclusion,
                "{status} {conclusion:?}"
            );
        }
    }

    #[test]
    fn malformed_rows_do_not_mask_other_evidence_or_create_success() {
        let mut bad_gate = gate("passed");
        bad_gate["stale_reason"] = json!(false);
        let got = loaded(&evidence(
            vec![bad_gate, gate("passed")],
            "available",
            vec![
                json!({"name":"bad", "status":"completed", "conclusion":true}),
                json!({"name":"good", "status":"completed", "conclusion":"success"}),
            ],
        ));
        assert_eq!(got.checks.len(), 2);
        assert_eq!(got.checks[0].name, "checks");
        assert_eq!(got.checks[1].name, "good");
        assert_eq!(
            got.notices
                .iter()
                .filter(|n| n.contains("could not be read"))
                .count(),
            2
        );

        for bad in [
            json!({"availability":"available"}),
            json!({"availability":"available", "runs":false}),
        ] {
            let mut data = evidence(vec![], "available", vec![]);
            data["checks"] = bad;
            let got = loaded(&data);
            assert!(got.checks.is_empty());
            assert!(got.notices.iter().any(|n| n.contains("valid CI run list")));
        }
    }

    #[test]
    fn detail_links_are_https_and_cannot_carry_credentials_or_controls() {
        let allowed = "https://buildkite.com/acme/builds/1?job=lint#step-2";
        assert_eq!(safe_details_url(allowed).as_deref(), Some(allowed));
        assert!(safe_details_url("https://ci.example/path%20with%20spaces").is_some());
        for unsafe_url in [
            "http://ci.example/run",
            "file:///tmp/report",
            "javascript:alert(1)",
            "/relative",
            "//ci.example/run",
            "https://user:secret@ci.example/run",
            "https://user@ci.example/run",
            "https://@ci.example/run",
            "https:ci.example/run",
            "https:/ci.example/run",
            "https:////ci.example/run",
            "https://ci.example/run\n",
            "https://ci.example/\tbad",
            "https://ci.example/path with space",
            "https://ci.example\\run",
            "https://ci.example/%0a",
            "https://ci.example/%0D",
            "https://ci.example/%00",
            "https://ci.example/%7f",
            "https:///",
        ] {
            assert_eq!(safe_details_url(unsafe_url), None, "{unsafe_url:?}");
        }
        let mut with_url = gate("passed");
        with_url["value"] = json!({"web_url":"javascript:alert(1)"});
        let got = loaded(&evidence(
            vec![with_url],
            "available",
            vec![json!({
                "name":"CI", "status":"completed", "conclusion":"success", "details_url":"https://user:secret@ci.example/run"
            })],
        ));
        assert!(got.checks.iter().all(|c| c.url.is_none()));
    }

    #[test]
    fn only_full_immutable_hashes_bind_evidence_and_existing_overview_is_preserved() {
        let hash256 = "a".repeat(64);
        assert!(same_revision(Some(&hash256), &hash256));
        assert!(same_revision(Some(&HEAD.to_uppercase()), HEAD));
        assert!(!same_revision(Some("main"), "main"));
        assert!(!same_revision(Some(""), ""));
        let mut got = Overview {
            description: Some("Description".into()),
            captured_at: Some(17),
            notices: vec!["Local drafts stay local.".into()],
            ..Overview::default()
        };
        apply(&mut got, &evidence(vec![], "available", vec![]), HEAD);
        assert_eq!(got.description.as_deref(), Some("Description"));
        assert_eq!(got.captured_at, Some(17));
        assert!(got.notices.iter().any(|n| n == "Local drafts stay local."));
        assert!(
            got.notices
                .iter()
                .any(|n| n.contains("no external CI runs"))
        );
    }
}
