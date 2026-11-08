# Entire support

Diffz opens a repository Change within an [Entire](https://entire.com) Trail
as a read-only review. It uses your Entire CLI login to load the change and
its diff, and `curl` to fetch source context without reformatting the file.
The initial integration covers Entire-hosted repositories and GitHub mirrors.
It does not yet cover every Entire feature; the remaining work is listed
under [Roadmap](#roadmap).

The initial opening workflow is tracked in
[#176](https://github.com/zzwong/diffz/issues/176), and authenticated diff and
source loading in [#177](https://github.com/zzwong/diffz/issues/177).

## Setup

Install the [Entire CLI](https://docs.entire.com/installation) and `curl`, and
make both available on the path used to launch Diffz. With Homebrew on macOS:

```sh
brew install --cask entireio/tap/entire
curl --version
entire version
entire login
diffz --doctor
```

On Linux, install `curl` with your distribution's package
manager, then use Entire's official installer:

```sh
curl -fsSL https://entire.com/install.sh | bash
entire version
entire login
diffz --doctor
```

Use `entire login --device` where a browser cannot open directly. The adapter's
reference CLI version is **v0.12.2**; older commands and response shapes may
differ. `--doctor` checks external tools, not access to a particular repository.
A successful login must still have permission to read the selected repository.
Opening a bare project Trail also needs permission to inspect that project;
an explicit Change URL uses repository reads instead.

Diffz reads your existing login context. It does not run `entire enable`, install
capture hooks, create a mirror, clone a repository, or publish a Change. You do
not need to enable session capture to review a Change.

## Open a Change

Use the URL for one Change in a Trail:

```sh
# A repository hosted on Entire.
diffz --trail 'https://entire.com/et/acme/-/trails/42/changes/widget/7'

# A GitHub repository mirrored on Entire.
diffz 'https://entire.com/gh/acme/-/trails/42/changes/widget/7'

# Inspect the loaded snapshot without opening a window.
diffz --inspect --trail 'https://entire.com/et/acme/-/trails/42/changes/widget/7'
```

The examples use placeholder project, repository, and review numbers. Copy the
actual Change URL from Entire. The Open panel and clipboard opening recognize
the same addresses. Existing-window handoff and saved-review resume use the
same source as a terminal launch.

| URL component | Meaning |
| --- | --- |
| `entire.com` | Current Entire web host. Older `entire.io` links are also accepted. |
| `et/acme` | An Entire project named `acme`. |
| `gh/acme` | A GitHub owner namespace mirrored on Entire. |
| `-/trails/42` | Project Trail number 42. |
| `changes/widget/7` | Change number 7 in repository `widget`. |

Earlier repository-scoped URLs such as
`https://entire.com/gh/acme/widget/trails/7` also identify one Change. Use a
clean HTTPS link: credentials, custom ports, and query parameters are rejected.
If a copied link includes tracking or redirect query parameters, remove that
query before opening it. This integration targets Entire's production hosts;
custom or staging API origins are not accepted.

A project Trail and its repository Changes have different numbers. A Trail can
contain several Changes, even in different repositories. A URL such as
`https://entire.com/et/acme/-/trails/42` opens directly only when the response
identifies one accessible Change without reporting a partial result. Otherwise
Diffz asks for a specific Change and lists available Change URLs. It does not
silently choose the first repository or combine unrelated diffs.

An Entire Change in a GitHub mirror is still opened through Entire. Its number
is not treated as a GitHub pull request number. This also matters for
Entire-only branches that have not been forwarded to GitHub.

### Local repositories

An existing local checkout works with Diffz's normal Git commands, regardless
of where it is hosted:

```sh
diffz --git /path/to/checkout --base main
diffz --staged /path/to/checkout
diffz --worktree /path/to/checkout
```

These commands read local Git data. They do not fetch or require an Entire
login. Cloning, fetching, and configuring an `entire://` remote remain normal
Git/Entire CLI operations you perform separately.

## Scope of this integration

- Load one selected Change, with its title, state, and diff.
- Use the project and repository's assigned API regions, rather than assuming
  that all repositories live in the login account's home region.
- Keep the loaded review as a snapshot and use the existing refresh workflow
  to look for later work.
- Show gates and external CI checks included in the selected Change's detail,
  with their saved status, non-blocking labels, and safe HTTPS details links.
  Missing, stale, or unknown evidence does not become a passing result.
- Summarize up to ten indexed sessions linked to the captured comparison, with
  their IDs, agents, models, and activity times. Partial or inaccessible session
  history is identified; transcripts are not fetched.
- Read source context for the selected snapshot without rewriting JSON file
  formatting during transport.
- Keep comments as local drafts and allow local export. Entire publishing is
  unavailable, including when GitHub or GitLab write flags are enabled.

Open, draft, closed, and merged Changes can be read when Entire supplies the
complete immutable revision set. A terminal Change does not need its former
branch to exist. An unindexed or incomplete snapshot reports a loading error
rather than substituting the current branch tip.

Standalone Entire commit and compare URLs, immutable source-line permalinks,
a multi-Change picker, remote review threads and findings, full review/CI
history, and checkpoint transcripts are not part of this initial workflow.
`--compare` continues to accept the GitHub and GitLab forms described in the
main README. The absence of session or review details in Diffz is not evidence
that Entire has none.

## Authentication and data routing

This section records the integration contract for maintainers. Entire has a
control plane called Core and regional data APIs. A project Trail and a
repository Change need not share the same API region.

1. An explicit Change URL already identifies the repository and Change number,
   so it uses repository reads directly. Only a bare project Trail first calls
   Core's `GET /api/v1/projects/resolve/{host}/{project}`. That path validates
   the returned project reference and its `apiUrl`, `primaryProcessingCell`,
   and `region`, then reads
   `GET /api/v1/{host}/{project}/trails/{number}` on the assigned project host.
   It selects one Change only when the result is explicitly complete and
   unambiguous.
2. Resolve the repository with the exact forge-qualified Core directory filter
   `GET /api/v1/repos?filter=et/PROJECT/REPO` or its `gh/OWNER/REPO` equivalent.
   Verify the returned namespace/provider and canonical repository name. A
   name-only lookup is insufficient: a native `et` repository and a `gh` mirror
   can share the same owner/repository spelling. This bounded lookup does not
   enumerate the account or follow pages; an unexpected `nextPageToken` or
   `truncated: true` fails as incomplete instead of selecting a partial result.
3. Match `repos[].primaries.processing` to exactly one `placements[].id`,
   require that placement to be ready, and use its `apiBaseUrl`. Read the
   repository Change at `/api/v1/repos/{placement}/trails/{number}`. Validate
   its identity and any selected parent Trail. A current Change URL must match
   the returned parent number and namespace. A legacy URL with a discoverable,
   valid parent is canonicalized to the current Change URL; an absent parent
   does not prevent a legacy repository-scoped read.
4. Use the ID form each regional operation requires. Git-data paths such as
   `/api/v1/git/repos/{repoId}/diff` require a placement ULID. A project ID,
   project Trail ID, repository Change number, and public `repo_…` repository
   identity are not interchangeable with that ULID. Recheck the repository,
   account, Change pins, and any selected project membership before installing
   the snapshot.

The primary can have `status: "ready"`. Selecting any ready mirror, or looking
for `status: "processing"`, does not establish the processing primary. If the
assigned host or identity is missing, fail with an actionable routing error;
do not fall back to a guessed US or home-region endpoint.

`entire api --to core` uses the selected login's Core. `--to cell` alone uses
the account's home cell; the adapter must supply the validated repository or
project host through `ENTIRE_API_BASE_URL` for region-specific reads. Paths
remain relative to that host. It must not forward credentials to arbitrary
hosts derived from pasted URLs or follow cross-host redirects with a bearer.

The user's account and repository permissions govern reads, including private
repositories. The published Git-data API requires a bearer with pull access;
a public web page does not establish an anonymous API contract. Diffz uses the
own-account workflow, which does not require registering an Entire plugin.

### Diff and source context

The adapter uses these published Git operations on the selected region:

| Operation | Path and query |
| --- | --- |
| List changed files | `/api/v1/git/repos/{repoId}/compare?base=BASE&head=HEAD` |
| Read the unified diff | `/api/v1/git/repos/{repoId}/diff?base=BASE&head=HEAD` |
| Read a source file | `/api/v1/git/repos/{repoId}/raw/{PATH}?ref=REVISION` |

Encode each query value and the raw-file path. Resolve the revisions used by a
review and retain their immutable identity so later branch movement does not change source
context underneath the snapshot. The Change detail exposes `base_sha`,
`head_sha`, and `merge_base_sha`; preserve those pins and recheck the Change's
identity around loading. Historical Changes can retain pinned revisions after
their branch has disappeared. A `compare` response with `truncated: true` cannot
be presented as an exhaustive changed-file list.

Metadata and unified diff text can pass through `entire api`. Source files
need a different path: in v0.12.2, `entire api` pretty-prints every response
whose body is valid JSON, even when the endpoint is returning a source file.
Using that output would alter whitespace and trailing newlines in JSON source.
Diffz obtains the login bearer through `entire auth token` and passes it to
`curl` through standard input to fetch the raw response. The token is not a
command-line argument, URL parameter, or part of a stored snapshot. Fetching
context must preserve the source bytes before the reader processes them.

Entire uses problem details to distinguish several Git-read failures that all
have HTTP status 404. A missing ref, a temporarily missing object on a replica,
an empty/deleted repository, and insufficient access must not all become
"repository does not exist." A missing file at a valid ref is also distinct.
The published endpoint documentation describes those problem types and the
possibility of replication lag.

### Gates and CI context

The Change detail's `mergeability` object carries `head_sha`, `gates`, and
`checks`. The gate/CI overview reads this object without making extra network
requests. Its head must match the snapshot before any result is used. A gate
must have current head and evaluation pins before a passing outcome is shown;
stale reasons, an in-flight evaluation, inconsistent outcomes, and unknown
statuses remain pending or unavailable.

CI runs are meaningful only when `checks.availability` is `available`. A
completed run with the explicit `success` conclusion is shown as passing;
neutral, skipped, cancelled, and unknown results are not converted to success.
Missing required checks can appear as queued placeholders even when CI evidence
is unavailable. Diffz preserves those pending placeholders and explains the
availability limit. Gate and check results do not create a human review
approval or a locally inferred merge decision.

### Session summaries

An optional read requests
`/api/v1/repos/{placement}/sessions?base=MERGE_BASE&head=HEAD&limit=10` with the
snapshot's immutable revisions. It happens before the final identity rechecks.
The overview keeps at most ten summaries with the session ID, agent, model,
and last activity time (or its start time when last activity is absent).
It does not retain prompts, session display names, or full response objects,
and it does not fetch transcripts.

A continuation cursor, incomplete or unavailable history, missing branch
coverage, or malformed/duplicate records produces an explicit notice. The
initial integration does not page through all sessions. An empty complete
result means no indexed sessions are linked to this range; it does not prove
that no agent worked on the code. A failed or unauthorized optional session
read leaves the diff usable with an availability notice; cancelling the
overall load still cancels it.

## Verification

Automated contract tests can verify URL parsing, subprocess arguments, routing
responses, diff loading, raw-file preservation, and write refusal without a
real account. They cannot establish live API compatibility, private-repository
permissions, or a working native desktop session.

The initial integration has **not been qualified against an authenticated
live Entire repository or a native Linux/macOS GUI session**. Record those
observations in [#182](https://github.com/zzwong/diffz/issues/182) before claiming
full support. Useful live cases are:

- An Entire-hosted repository and a GitHub mirror, including an Entire-only
  branch; public and private access.
- A repository whose processing region differs from the login's home region.
- A single-Change Trail, a multi-Change Trail, and a Trail with partially
  accessible Changes.
- Branch movement between refreshes, renamed and deleted files, JSON with
  deliberate formatting, CRLF, missing trailing newlines, and a large diff.
- Missing `entire` or `curl`, expired login, revoked repository access,
  unavailable processing placement, and retryable replica lag.
- CLI, Open panel, clipboard, existing-window handoff, refresh, local drafts,
  export, and saved-review resume on both supported desktop platforms.

## Roadmap

[Full support is tracked in #175](https://github.com/zzwong/diffz/issues/175).
The following areas remain separate from opening one Change:

| Area | Tracking | Completion criteria |
| --- | --- | --- |
| Commit, compare, and source links | [#180](https://github.com/zzwong/diffz/issues/180) | Establish canonical URL and immutable source-permalink contracts; support root/merge commit semantics, encoded refs, and complete diff coverage. |
| Multi-Change Trails | [#178](https://github.com/zzwong/diffz/issues/178) | Let the user choose among repository Changes; retain explicit partial-access state and link each selection to its parent Trail. |
| Review and CI context | [#178](https://github.com/zzwong/diffz/issues/178) | Basic saved gate/CI status is included. Add remote discussions, agent findings, reviewer approvals, and richer review/check history with accurate line/side mapping. |
| Checkpoints and sessions | [#179](https://github.com/zzwong/diffz/issues/179) | Bounded session summaries are included. Add a typed, paginated session/checkpoint view, commit linkage, current and legacy storage, and support for a separate private checkpoint repository. Retrieve transcripts deliberately. |
| Review publishing | [#181](https://github.com/zzwong/diffz/issues/181) | Add an explicit write capability, preview, correct permission checks, stale-head validation, and outbox reconciliation before offering remote comments or approvals. |
| Live and desktop qualification | [#182](https://github.com/zzwong/diffz/issues/182) | Complete the authenticated and native cases above and record versions, identities, outcomes, and remaining limitations. |

## Research baseline and primary sources

Contract research was performed on **2026-11-05** against the public
documentation and OpenAPI specification, plus **Entire CLI v0.12.2**, commit
`c4f103d8dfed1f1549db9b2c793f2d7e10f65eb8`. The CLI tag is pinned; the web
documentation and server specification can change. These sources establish
the intended contract, not a claim of authenticated runtime verification.

| Subject | Primary source |
| --- | --- |
| Installation and login | [Installation](https://docs.entire.com/installation), [Authentication](https://docs.entire.com/guides/api/authentication) |
| Hosting and mirror identities | [Repositories](https://docs.entire.com/guides/repositories/repository), [Mirrors](https://docs.entire.com/guides/repositories/mirrors) |
| Project Trail / repository Change model | [Trail CLI reference](https://docs.entire.com/cli-reference/trail), [v0.12.2 architecture](https://github.com/entireio/cli/blob/v0.12.2/docs/architecture/project-trails-cli.md) |
| Assigned-region routing | [Regions and routing](https://docs.entire.com/guides/api/regions-and-routing), [v0.12.2 project resolver](https://github.com/entireio/cli/blob/v0.12.2/cmd/entire/cli/project_trail_target.go) |
| API discovery and schemas | [API guide](https://docs.entire.com/guides/api/getting-started), [documentation index](https://docs.entire.com/llms.txt), [regional OpenAPI](https://docs.entire.com/openapi/regional.json) |
| Git diff and raw contents | [Unified diff](https://docs.entire.com/api-reference/git/repositories/get-unified-diff), [raw file](https://docs.entire.com/api-reference/git/repositories/get-raw-file-by-path) |
| CLI output and routing behavior | [v0.12.2 `api_cmd.go`](https://github.com/entireio/cli/blob/v0.12.2/cmd/entire/cli/api_cmd.go) |
| Session metadata and coverage | [Sessions in a repository](https://docs.entire.com/api-reference/sessions/sessions/sessions-in-a-repository) |
| Checkpoint storage and privacy | [Session storage](https://docs.entire.com/guides/checkpoints/checkpoint-storage), [separate checkpoint repository](https://docs.entire.com/guides/checkpoints/store-checkpoints-in-another-repo) |

### Typed Change discovery (adapter API)

`EntireReader::choices` reads a bare project Trail into `TrailChoices` without
loading a diff. It exposes readable repository/Change labels and the explicit
partial-access marker. Empty or partially visible membership is not evidence
that the Trail has no other Changes. Duplicate identities and malformed rows
are rejected rather than silently omitted.

`EntireReader::snapshot_choice` opens one index from that immutable result. It
rechecks the account, project identity/routing, selected Change ID and repository
placement before diff capture and retains the normal end-of-capture identity
checks. Callers must pass the typed result, not reconstruct a choice solely from
its display URL. An account change or replaced/removed choice requires fresh
discovery. These APIs do not publish reviews or retain authentication tokens.

This is the adapter foundation for #178. The desktop Open panel and CLI still
use the existing opening behavior; a selectable multi-Change UI is not yet wired
through the provider and service interfaces. Live qualification remains #182.
