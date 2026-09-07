# Synthetic regression corpus

This corpus contains no material from private repositories. `catalog.json` is the
source of truth for IDs, input paths, byte hashes, and expected behavior. URL samples
use `example.invalid` and must never be fetched. Synthetic commit and review IDs are
not valid targets for remote operations.

Each source pair includes exact before and after bytes plus a patch made by Git's
`diff --no-index`. When the bundle was assembled, `git apply --check` accepted each
patch against its before file. Hand-written metadata and malformed or combined cases
are marked clearly. These files feed a future strict parser; they do not show that one
already exists.

The set covers Markdown prose above 20k bytes, a URL above 50k bytes, Unicode,
emoji, bidi, CRLF and LF, changes only at the final newline, Markdown hard breaks
and tabs, uneven split wrapping, quoted paths, a source line over 256 KiB, 10,000
changed source lines, mode-only and rename-only edits, malformed and combined diffs,
layout-event oracles, review-publication events, and advanced target-branch rules.

Event JSON describes expected results for a future test harness. A package check that
only validates the JSON cannot show that the UI, source map, network adapter, or outbox
meets those expectations. Production layout must use actual GPUI font shaping, rather
than Python wrapping or character counts.
