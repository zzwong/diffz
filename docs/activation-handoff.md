# Ephemeral Wayland activation context

A launcher may give a Diffz invocation `XDG_ACTIVATION_TOKEN`. Diffz captures
and removes it at process entry, before starting its own threads. An admitted
token moves to the intended detached Diffz child, the initial GPUI window, or
one existing-window handoff. Helper/provider processes do not inherit it.

The handoff stays **version 1**. Its optional `activation_token` field is ignored
by old receivers; absent/null metadata preserves tokenless behavior. An Ack
still reports review acceptance/refusal, not activation support or focus. The
existing Ack version behavior is unchanged. Once a write is attempted, the
sender never restores or forwards that token again, including partial writes,
lost acknowledgements, a replacement owner, or a replacement child. This is
not an exactly-once review-delivery guarantee across a crash before Ack.

## Application limits and privacy

- Admitted tokens are nonempty UTF-8, at most **4,096 decoded bytes**, with no
  NUL. Accepted whitespace, Unicode and escaping are not normalized. This is a
  Diffz policy, **not an XDG protocol limit**. It also deliberately bounds cold
  startup: an oversized token is discarded while normal review opening proceeds.
- A frame is at most **65,536 serialized bytes including its terminal newline**.
  A capped prefix without its newline is invalid. When metadata alone causes
  overflow, the sender constructs one tokenless frame before the only write.
  An oversized base request fails before any write; nothing is truncated.
- Parseable unusable metadata becomes absent. Malformed or parser-limit-exceeding
  envelopes receive fixed errors with line/column, never rejected field values.
- Tokens are not review state, CLI arguments, status JSON, or diagnostics. The
  GPUI graphical-environment Debug carrier redacts token presence, including
  nested `WindowingRequest` formatting. Low-level Wayland protocol tracing can
  expose arguments; do not collect it with genuine launcher tokens.

## Existing-window activation

The listener timestamps a connection immediately after `accept`, before reading
or validation. The bridge has capacity one: one active handoff, one queued, and
at most one listener-owned handoff blocked on enqueue. This does not bound the
kernel backlog, external senders, or transient parser/serialization allocations.

At the actual window update, admitted context is fresh at **at most two seconds
since accept**. This includes read, validation, bridge and UI delay. It excludes
pre-accept backlog, launch-lock waits and token mint age; it is not a
launcher-to-focus bound or evidence of compositor validity.

| Context / target capability | Immediate action, before save wait | Later generic activation |
| --- | --- | --- |
| Fresh, Wayland activation global present | Submit exact token once to the existing target surface | Suppressed |
| Locally expired, capability present | Drop token, no submission | Suppressed |
| Fresh or expired, unsupported backend/global absent | Drop token | Original point after save wait |
| Absent or locally rejected metadata | None | Original point after save wait |

Suppression is selected by capability, not submission success. A false return
after a true capability query never re-enables fallback. The external path does
not mint a self-token, alter another pending URI/path activation, create a new
connection, change compositor policy, or retry. Ordinary tokenless activation
is unchanged. Existing save/draft protection, write-permission refusal, source
opening and reply decision points remain authoritative.

The sender can time out while a queued request is eventually processed. Bounded
backpressure preserves Ack meaning, not latency or end-to-end freshness.

## Verification boundaries

Pure tests use synthetic values and fake sinks to check byte transport, ownership,
framing, dispatch, startup lifetime and per-handoff policy. Policy event-list
markers model the caller's save/open/reply sequence; the full UI sequence and
preservation of unrelated Wayland pending activation are source-reviewed, not
runtime-qualified by those helpers. App subprocess tests exercise real
entry-point environment removal. None of these is native focus evidence.
The ordinary Linux/macOS CI graph also compiles authentic patched dependencies;
explicit dependency unit-test commands are required because workspace tests
alone do not run those crates' tests:

```sh
python3 scripts/qualify-gpui-activation.py
```

The script stages the unchanged vendor source as a workspace member under
`target/`, because Cargo cannot select features on an external workspace
dependency. It retains the app's patches and profiles. Offline resolution may
prune the copied lock, but rejects any new package, version, source or checksum;
the app lock is unchanged. It runs all four activation tests and verifies that
each backend check excludes the opposite backend and test support.

Record exact commits and actual outcomes in the PR. GPUI core's own-checkout tests,
`bash scripts/check.sh core`, `bash scripts/check.sh native`, and native X11/macOS
handoff smoke checks remain separate obligations.

Release focus/attention support is **not qualified by these tests**. That needs
observations with genuine launcher tokens in separately authorized disposable
compositor sessions, with the release app ID `io.github.zzwong.Diffz`, without
raw-token logs or compositor policy changes. Record backlog, locally expired
context and compositor-rejected context separately. Never use a user's active
desktop or treat a fake token/submission count as successful native focus.
