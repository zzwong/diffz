# Extensions: design draft

Status: draft, not implemented.

Diffz should be extensible at its core. Languages, themes, and anything that
adds information to a review should arrive through typed contracts, and the
built-in features should use those same contracts. Extensions never render UI
themselves. They describe content, and diffz renders it with GPUI.

## Goals

- One typed contract per extension point, shared by built-ins and third parties.
- Themes that can restyle every semantic surface, not just supply a palette.
- Languages added as data, without rebuilding diffz.
- Extensions that annotate a review: lint results, coverage, ownership, notes.
- Extension work never blocks rendering or input.

## Non-goals

- Extensions that draw arbitrary GPUI elements. GPUI's API is generic and
  closure-heavy, changes between snapshots, and has no stable ABI. Exposing it
  would freeze our toolkit version and let one slow extension stall the frame.
- Native `dylib` plugins. Rust has no stable ABI, so every toolchain bump
  would break them, and they cannot be sandboxed.
- A marketplace or network installer. Extensions are directories on disk for now.

## Layers

| Layer | What it carries | Runs code | Milestone |
| --- | --- | --- | --- |
| Registries | Rust traits inside the workspace | Built-ins only | M1 |
| Theme contract | `theme.toml` skin and syntax tokens | No | M2 |
| Data extensions | Manifest, tree-sitter grammars, queries, themes | No | M3 |
| Annotations | Typed annotations on files, hunks, lines | Built-ins first | M4 |
| Code extensions | WASM components against a WIT world | Yes, sandboxed | M5 |

Each layer is useful on its own. M1 and M2 change no user-facing behaviour
except richer themes.

## M1: registries

Today each extension point is hardwired:

- `syntax.rs` compiles 20 grammars through the `grammar!` macro and maps file
  extensions in a `match` inside `language_for_path`.
- `palette.rs` builds Omarchy's directories and the `"current"` reference
  into `theme_dirs` and `resolve_in`.
- `Services` keeps `gh`, `glab`, and local git as fixed fields.

M1 introduces traits in `diffz-core` (`crates/core/src/registry.rs`) and a
`Registry` that the app builds at startup and hands to the UI. The built-ins
become ordinary implementations.

```rust
pub trait LanguageProvider: Send + Sync {
    /// The language name for `path` and a priority. The highest priority wins;
    /// ties go to the provider registered first.
    fn claim(&self, path: &str) -> Option<(&str, u8)>;
    fn highlight(&self, path: &str, source: &str, cancel: &AtomicUsize) -> Option<Vec<Vec<Span>>>;
}

pub struct ThemeEntry {
    pub label: String,
    pub reference: String, // what settings and `--theme` store
    pub path: PathBuf,     // the colors.toml, watched while active
}

pub trait ThemeSource: Send + Sync {
    fn list(&self) -> Vec<ThemeEntry>;
    fn resolve(&self, reference: &str) -> Option<ThemeEntry>;
}
```

The registry owns the rules that apply to every provider: the 256 KB
highlight budget, cancellation, and the plain fallback for unclaimed files.
It also handles path references (`/…`, `~/…`) itself, before asking sources.

Built-in implementations:

- `BuiltinGrammars`: the 20 compiled tree-sitter grammars at priority 0,
  claiming nothing without the `syntax` feature.
- `OmarchyCurrent`: the live `~/.local/state/omarchy/current/theme`, selected
  as `current` or `omarchy`.
- `ThemeDirectories`: named themes under `~/.config/diffz/themes`,
  `~/.config/omarchy/themes`, and `/usr/share/omarchy/themes`, with the
  earlier directory winning when names repeat.

Theme references are unchanged, so saved settings and `--theme` keep working.
Namespaced references such as `omarchy:current` wait until a source exists
that is not file-backed.

Review providers moved behind traits in the same milestone; see
[Review providers](#review-providers). Annotators join the registry in M4.

`Token` and `Span` stay as they are. The closed `Token` enum is the contract
between highlighters and themes, and it is deliberately small.

## M2: theme contract

`Skin` in `crates/ui/src/theme.rs` has 17 semantic colours, all derived from
the 25 palette keys by `Skin::from_palette`. Syntax colours are fixed by
`Skin::token`, which folds 16 tokens onto 7 skin colours.

M2 moves the contract into core as `SkinSpec`, holding `Rgb` values so core
stays free of GPUI types. A theme may then supply a `theme.toml` next to or
instead of `colors.toml`:

```toml
extends = "colors.toml"   # optional; unset keys derive from this palette
mode = "dark"

[skin]
base = "#151820"
added = "#18332a"
added_word = "#28583c"
border = "#303848"

[syntax]
keyword = "#88b4ff"
string = "#81c995"
comment = { color = "#9ba5b5", italic = true }

[metrics]
radius = 6
density = "compact"        # compact | normal | roomy
```

Rules:

- Any key may be omitted. Missing skin keys come from `Skin::from_palette` on
  the `extends` palette, or from the built-in dark or light skin.
- Unknown keys produce a warning, not an error, so newer themes load on older
  builds.
- A bare `colors.toml` keeps working unchanged. Omarchy themes need nothing new.
- Syntax entries accept a colour or a table with `color`, `italic`, `bold`.

This is the "restyle everything" half of customization and needs no code
execution at all.

## M3: data extensions

An extension is a directory under `~/.config/diffz/extensions/<id>/`:

```
zig/
  extension.toml
  grammars/zig.wasm
  queries/zig/highlights.scm
  queries/zig/injections.scm
  themes/zig-dark/theme.toml
```

```toml
id = "zig"
name = "Zig"
version = "0.1.0"
diffz = "0.2"              # contract version this extension targets

[[languages]]
name = "zig"
grammar = "grammars/zig.wasm"
queries = "queries/zig"
extensions = ["zig", "zon"]

[[themes]]
path = "themes/zig-dark"
```

Grammars load through tree-sitter's `wasm` feature, so a grammar is data and
runs inside tree-sitter's own sandbox. Query capture names map onto `Token` by
the same `recognized_names` table the built-ins use. Captures with no mapping
stay unhighlighted.

Each manifest becomes one `LanguageProvider` and one `ThemeSource` in the
registry. An extension's grammar outranks a built-in for the same extension,
so users can replace a bundled grammar.

## M4: annotations

Annotations are the "stretch" half: extensions add typed content to surfaces
diffz already renders.

```rust
pub struct Annotation {
    pub anchor: AnnotationAnchor,
    pub severity: Severity,        // Note | Info | Warning | Error
    pub presentation: Presentation, // Gutter | Inline | Underline | FileBadge
    pub title: String,
    pub body: Option<String>,      // markdown, rendered by the rich view
    pub source: String,            // extension id, shown in the UI
}

pub enum AnnotationAnchor {
    File { path: String },
    Lines { path: String, side: Side, start: u32, end: u32 },
    Range { path: String, side: Side, line: u32, bytes: Range<u32> },
}

pub trait Annotator: Send + Sync {
    fn id(&self) -> &str;
    fn annotate(&self, request: &AnnotateRequest, cancel: Cancellation)
        -> Result<Vec<Annotation>, ServiceError>;
}
```

Anchors use repository paths and line numbers, not `FileId`, so they mean the
same thing to diffz and to an external tool. The host maps them onto
`SourcePoint` and drops any that fall outside the snapshot, counting them in a
warning instead of failing.

`AnnotateRequest` carries the snapshot title, the `RemoteTarget` if any, and
each changed file's path, old path, status, and hunk ranges. Full file
contents are fetched on demand through the host, because most annotators only
need a few files.

Execution follows the rule on `WorkbenchServices`: annotators run on the
bounded worker pool, never during update or render. Results are cached by
snapshot id and annotator version. A newer navigation generation discards
stale results, as `NavigationClock` does for scrolling.

Surfaces, in the order they would ship:

1. Gutter markers and an inline row under the anchored line in the viewport.
2. File badges in the file tree, with counts by severity.
3. An annotations list in the inspector, filterable by source and severity.

The first annotator is a built-in, which proves the contract before any WASM
host exists. A good candidate is surfacing `Snapshot::warnings` and patch
report problems at the lines they refer to.

## M5: code extensions

Code runs as WASM components under wasmtime, against a versioned WIT world.
WIT gives the strongly typed boundary: bindings are generated for the host in
Rust and for guests in Rust, Go, JS, Python, and others.

```wit
package diffz:extension@0.1.0;

interface types {
  enum side { old, new }
  enum severity { note, info, warning, error }
  enum presentation { gutter, inline, underline, file-badge }

  record line-range { start: u32, end: u32 }

  variant anchor {
    file(string),
    lines(tuple<string, side, line-range>),
    range(tuple<string, side, u32, tuple<u32, u32>>),
  }

  record annotation {
    anchor: anchor,
    severity: severity,
    presentation: presentation,
    title: string,
    body: option<string>,
  }

  enum file-status { added, modified, deleted, renamed }

  record hunk { old-start: u32, old-count: u32, new-start: u32, new-count: u32 }

  record changed-file {
    path: string,
    old-path: option<string>,
    status: file-status,
    hunks: list<hunk>,
  }

  record review {
    title: string,
    base: option<string>,
    head: option<string>,
    files: list<changed-file>,
  }
}

interface host {
  use types.{side};
  source: func(path: string, side: side) -> result<string, string>;
  setting: func(key: string) -> option<string>;
  log: func(message: string);
}

interface annotator {
  use types.{review, annotation};
  annotate: func(review: review) -> result<list<annotation>, string>;
}

world extension {
  import host;
  export annotator;
}
```

Sandboxing and limits:

- No filesystem, network, or process access by default. The manifest declares
  any capability it wants, and diffz asks once per extension version.
- Each call runs with epoch interruption and a wall-clock budget, two seconds
  to start with. A timed-out call yields no annotations and a status message.
- Memory is capped per instance.

Commands and declarative panels come after annotators, in the same world:
commands export a list of `{id, title, default-key}` and a `run` function
whose result is annotations or a panel; panels are a small tree of list,
tree, markdown, table, and button nodes. They wait until annotations have
shown what the contract needs.

## Versioning

- The WIT package and the manifest `diffz` key share one contract version.
- Additive changes bump the minor version. Diffz loads extensions built for
  any older minor of the same major.
- Theme and manifest parsing ignore unknown keys with a warning.

## Review providers

Providers stay native Rust in this repository, behind a typed trait. They are
not part of the WIT world.

- A provider needs credentials, network, and usually a CLI such as `gh` or
  `glab`. A WASM provider would need every capability, so the sandbox would
  buy nothing.
- Publishing is the riskiest path in diffz: the outbox's idempotent publish
  and reconcile must survive a crash mid-post. New providers should arrive as
  reviewed pull requests with fixture tests, like `tests/gitlab.rs`.

Core never names a host. Everything host-specific sits in two traits,
implemented once per provider in `diffz-adapters` (`github.rs`, `gitlab.rs`):

- `ReviewRules` (core, no I/O): names and Open panel text, the write-flag
  name, supported verdicts, the reopen address, line links, checks before a
  review is frozen, each comment's frozen position, the payload that is sent
  and fingerprinted, and whether remote reviews carry diffz's marker.
- `ReviewProvider` (adapters): its rules, plus `open`, `source`, and the
  `ReviewRemote` that sends and reconciles.

`Services::register(provider, writes)` adds one; `Services` keeps them in a
list and an outbox for each provider the user let publish. The UI reaches
rules through `WorkbenchServices::provider`, and `OpenRequest::Remote` names
the provider by id.

A remote target stores its provider as `ProviderId`, a string. GitHub and
GitLab keep the strings their old enum variants serialized as, so stored
snapshots and outbox entries are unchanged, and data from a provider this
build does not know still loads: it shows, but cannot refresh or publish.
Snapshot identity tags derive from the id (`snapshot-v1` for GitHub,
`snapshot-<id>-v1` otherwise).

A provider's payload bytes are part of the review fingerprint, so payloads
are serialized structs, never `json!` maps, whose key order depends on
serde_json's `preserve_order` feature. Golden tests in
`crates/adapters/tests/provider_compat.rs` pin targets, identities, payloads
and stored reviews, and run with and without that feature.

Gitea, Bitbucket, and others are left to community contributions: one module
implementing both traits and one `register` call. If out-of-tree providers
are ever needed, they should be executables speaking versioned JSON over
stdio, not WASM.

## Wasmtime

Tree-sitter's `wasm` feature depends on `wasmtime-c-api-impl` and re-exports
`tree_sitter::wasmtime`. Tree-sitter 0.27 pins wasmtime `^48`; wasmtime itself
is at 49. Depending on `wasmtime = "48"` with `component-model` enabled
unifies into one crate, so grammars and code extensions share one `Engine`.
Our wasmtime upgrades then follow tree-sitter's, about one major behind.

Moving from tree-sitter 0.25 to 0.27 is one call-site change: `highlight()`
gained an `encoding` argument. Core tests pass on 0.27.

Measured on a spike branch, release profile, 8 cores, the Rust grammar built
with `tree-sitter build --wasm` (1.1 MB), highlighting `viewport.rs`
(1933 lines):

| | Without wasmtime | With wasmtime |
| --- | --- | --- |
| Binary size | 47.4 MiB | 56.2 MiB (+18.5%) |
| Clean release build | 386 s | 482 s (+25%) |

| Runtime step | Cost |
| --- | --- |
| `Engine::new` | 0.2 ms |
| `WasmStore::new` | 18 ms |
| Compile one grammar | 59 ms |
| Highlight, native grammar | 25 ms |
| Highlight, wasm grammar | 31 ms (+20 to 25%) |
| Resident memory after loading one grammar | +16.5 MB |

Consequences:

- Startup is unaffected if the engine and store are created on first use of
  an extension grammar. Nothing in the built-in path touches wasmtime.
- Grammar compilation should be cached on disk, keyed by the grammar's hash
  and the wasmtime version, if 59 ms per grammar proves noticeable.
- `wasmtime-c-api-impl` needs `cmake` at build time. That is a new
  dependency for contributors, CI, and every distribution in `linux.md`.
  Putting wasm support behind a default-on cargo feature lets packagers opt out.
- Grammar authors need wasi-sdk; `tree-sitter build --wasm` downloads it
  (114 MB) on first use. Users never do.

## Open questions

- Linters usually need to run a process. A `process` capability is the
  obvious answer, but it weakens the sandbox to "whatever the user trusts".
- Whether 9 MB and 16 MB of resident memory are acceptable for users who
  never install an extension, or wasm support ships as an optional feature.
