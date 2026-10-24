//! Ephemeral per-handoff activation policy; never part of review state.
use std::time::{Duration, Instant};

const USE_BUDGET: Duration = Duration::from_secs(2);

// Expired is deliberately distinct from Absent after dropping the token bytes.
enum Context {
    Absent,
    Fresh(String),
    Expired,
}

pub(super) struct ActivationPolicy {
    suppress_generic: bool,
}

impl ActivationPolicy {
    /// Invoke at the original activation point after the save wait. Keeping this
    /// decision after token disposal prevents a later generic activation replay.
    pub(super) fn activate_later(self, activate: impl FnOnce()) {
        if !self.suppress_generic {
            activate();
        }
    }
}

/// Selects the policy retained after disposing of this handoff's token.
/// `now` must be sampled in the window update immediately before this call.
/// The age includes accepted-socket read/bridge/UI delay, but not listen backlog.
pub(super) fn activate_external(
    token: Option<String>,
    accepted_at: Instant,
    now: Instant,
    supported: bool,
    submit: impl FnOnce(String) -> bool,
) -> ActivationPolicy {
    let context = match token {
        None => Context::Absent,
        Some(token) if now.saturating_duration_since(accepted_at) <= USE_BUDGET => {
            Context::Fresh(token)
        }
        Some(_) => Context::Expired,
    };
    let suppress_generic = match context {
        Context::Absent => false,
        Context::Fresh(token) if supported => {
            // Suppression is selected by capability, not the submission result.
            // No retry or self-token follows a disappearing capability/rejection.
            let _ = submit(token);
            true
        }
        Context::Expired if supported => true,
        Context::Fresh(_) | Context::Expired => false,
    };
    ActivationPolicy { suppress_generic }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actual_use_boundary_and_disposition_table() {
        let accepted = Instant::now();
        for (token, age, supported, submitted, suppressed) in [
            (false, Duration::ZERO, true, false, false),
            (true, USE_BUDGET, true, true, true),
            (
                true,
                USE_BUDGET + Duration::from_nanos(1),
                true,
                false,
                true,
            ),
            (true, Duration::ZERO, false, false, false),
            (
                true,
                USE_BUDGET + Duration::from_secs(1),
                false,
                false,
                false,
            ),
        ] {
            let mut calls = 0;
            let suppress = activate_external(
                token.then(|| "synthetic opaque \n token".to_owned()),
                accepted,
                accepted + age,
                supported,
                |value| {
                    assert!(value == "synthetic opaque \n token");
                    calls += 1;
                    true
                },
            );
            assert_eq!(calls, usize::from(submitted));
            assert_eq!(suppress.suppress_generic, suppressed);
        }
    }

    #[test]
    fn submission_false_does_not_enable_a_later_generic_attempt() {
        let accepted = Instant::now();
        let suppress =
            activate_external(Some("synthetic".into()), accepted, accepted, true, |_| {
                false
            });
        assert!(suppress.suppress_generic);
    }

    #[test]
    fn read_bridge_and_ui_delay_counts_but_pre_accept_backlog_does_not() {
        let connected = Instant::now();
        let accepted = connected + Duration::from_secs(30);
        // This only proves the specified receiver-local budget, not mint freshness.
        assert!(
            activate_external(Some("synthetic".into()), accepted, accepted, true, |_| true)
                .suppress_generic
        );
        let mut submissions = 0;
        assert!(
            activate_external(
                Some("synthetic".into()),
                accepted,
                accepted + USE_BUDGET + Duration::from_nanos(1),
                true,
                |_| {
                    submissions += 1;
                    true
                }
            )
            .suppress_generic
        );
        assert_eq!(submissions, 0);
    }
    #[test]
    fn policy_models_external_before_wait_and_generic_at_original_point() {
        // The production policy methods are exercised here; save/wait/open/reply
        // markers model the caller's ordering, not a running native UI loop.
        let accepted = Instant::now();
        for (token, age, supported, submission_result, external, generic) in [
            (true, Duration::ZERO, true, true, true, false),
            (true, Duration::ZERO, true, false, true, false),
            (
                true,
                USE_BUDGET + Duration::from_nanos(1),
                true,
                true,
                false,
                false,
            ),
            (true, Duration::ZERO, false, true, false, true),
            (
                true,
                USE_BUDGET + Duration::from_nanos(1),
                false,
                true,
                false,
                true,
            ),
            (false, Duration::ZERO, true, true, false, true),
        ] {
            let mut events = Vec::new();
            let policy = activate_external(
                token.then(|| "synthetic".into()),
                accepted,
                accepted + age,
                supported,
                |_| {
                    events.push("external");
                    submission_result
                },
            );
            events.push("save-wait");
            policy.activate_later(|| events.push("generic"));
            events.extend(["take-handoff-or-refuse", "reply"]);
            let mut expected = Vec::new();
            if external {
                expected.push("external");
            }
            expected.push("save-wait");
            if generic {
                expected.push("generic");
            }
            expected.extend(["take-handoff-or-refuse", "reply"]);
            assert_eq!(events, expected);
        }
    }
}
