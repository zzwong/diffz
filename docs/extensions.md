# Extensions

Status: M1 to M5 are implemented. Deferred items are listed under each
milestone.

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

The contract lives in core (`crates/core/src/theme.rs`): `SkinSpec` holds the
17 semantic colours as `Rgb`, so core stays free of GPUI types, and `Theme`
adds the mode, an optional base palette, per-token syntax overrides, the files
it was read from, and warnings. The UI only converts `Rgb` to `Hsla`.

A theme folder may hold a `theme.toml` next to or instead of `colors.toml`,
and the folder prefers it:

```toml
extends = "colors.toml"   # optional; a palette relative to this file
mode = "dark"             # optional; defaults to the palette's, else dark

[skin]
added = "#18332a"
added_word = "#28583c"

[syntax]
keyword = "#88b4ff"
comment = "#9ba5b5"
```

- Any key may be omitted. Skin keys fall back to the `extends` palette's
  mapping, or to the built-in dark or light skin; syntax keys fall back to
  their skin colour.
- The palette also colours the widget layer; a theme without `extends` keeps
  the built-in widget theme.
- Unknown keys and sections are ignored with a warning in the status line, so
  newer themes load on older builds. Malformed colours are errors.
- An edit to `theme.toml` or to the palette it extends reloads the theme.
- A bare `colors.toml` works unchanged, so Omarchy themes need nothing new.

Deferred:

- **Bold and italic tokens.** Lines are shaped once with one font and
  highlighting only recolours them, so it can never move text. Per-token
  weight means reshaping per span and measuring its cost first.
- **Metrics** (`radius`, `density`). Corner radii are hard-coded at about
  fifteen call sites and spacing lives in layout constants, so these need a
  pass through the chrome before a theme can drive them.

This is the "restyle everything" half of customization and needs no code
execution at all.

## M3: data extensions

An extension is a folder under `$XDG_CONFIG_HOME/diffz/extensions/<folder>/`
(default `~/.config/diffz/extensions`) with an `extension.toml`:

```
zig/
  extension.toml
  grammars/zig.wasm
  queries/zig/highlights.scm      # required; injections.scm and locals.scm optional
  themes/zig-dark/theme.toml      # or colors.toml
```

```toml
id = "zig"                 # letters, digits, - and _; unique across extensions
name = "Zig"
version = "0.1.0"
diffz = "0.1"              # contract version the extension targets

[[languages]]
name = "zig"
grammar = "grammars/zig.wasm"
queries = "queries/zig"
extensions = ["zig", "zon"]

[[themes]]
path = "themes/zig-dark"
```

- Grammars are WebAssembly built with `tree-sitter build --wasm`, loaded
  through tree-sitter's `wasm` feature and run inside wasmtime's sandbox. Each
  compiles on first use; highlighters and their WASM stores are pooled.
- Query captures map onto `Token` through the same table as the built-in
  grammars; unmapped captures stay plain.
- An extension language outranks a built-in grammar for the same file
  extension, so an extension can replace a bundled grammar.
- Extension themes are listed by folder name after the user's theme
  directories, which win when names collide.
- Paths in a manifest must stay inside the extension folder.
- An extension is skipped, with a reason, when its manifest does not parse, its
  id repeats, or its `diffz` version has another major or a newer minor than
  this build. `diffz --doctor` lists extensions and problems and compiles every
  grammar; skipped extensions are also printed to stderr at startup.
- The `wasm` cargo feature, on by default, needs CMake to build wasmtime.
  Without it, extension themes still load and extension grammars are reported
  as unsupported.

`crates/core/tests/fixtures/extensions/toy` is a complete example with a tiny
grammar; its `source/` folder rebuilds the `.wasm`.

## M4: annotations

Annotations are the "stretch" half: annotators add typed content to surfaces
diffz already renders. The contract is in `crates/core/src/annotation.rs`:

```rust
pub enum Severity { Note, Info, Warning, Error }

pub enum Anchor {
    File { path: String },
    Lines { path: String, side: Side, start: u32, end: u32 },
}

pub struct Annotation {
    pub anchor: Anchor,
    pub severity: Severity,
    pub title: String,
    pub body: Option<String>,
    pub source: String,
}

pub trait Annotator: Send + Sync {
    fn id(&self) -> &str;
    fn annotate(&self, snapshot: &Snapshot, cancel: &Cancellation)
        -> Result<Vec<Annotation>, String>;
}
```

- Anchors use display paths and line numbers, not `FileId`, so they mean the
  same thing to diffz and to an external tool. The registry drops annotations
  whose file or lines are not in the snapshot and reports how many.
- Annotators run on a background worker after a review opens. A newer open
  cancels the run, and results for a snapshot that is no longer active are
  discarded. Failures appear in the status line.
- The built-in annotator, `diff check`, flags on added lines what
  `git diff --check` does: conflict markers (error), trailing whitespace and a
  space before a tab in indentation (warnings). Adjacent lines with the same
  finding merge into one range.

Surfaces:

- A severity-coloured bar at the left edge of each annotated line.
- Annotations covering the selected lines, listed in the line panel above the
  draft box; file annotations show with file comments.
- A severity-coloured count badge per file in the file tree.
- An **Annotations** tab in the inspector; choosing one reveals its line.

Deferred:

- **Inline rows** under the anchored line. The viewport has no row kind between
  diff lines (threads also open in a panel), so this needs a new row type
  through measurement, height indexing and scroll anchoring.
- **Byte-range anchors** with underlines. `DecorationRun` supports underlines,
  but a range anchor has no producer until code extensions can emit one.

## M5: code extensions

Code runs as WebAssembly components under wasmtime, against the WIT world in
`crates/core/wit/extension.wit`. WIT is the strongly typed boundary: the host
binds it with `wasmtime::component::bindgen!`, and guests in Rust, Go, JS,
Python and others generate bindings from the same file.

```wit
package diffz:extension@0.1.0;

interface annotator {
  use types.{review, annotation};
  annotate: func(review: review) -> result<list<annotation>, string>;
}

world extension {
  export annotator;
}
```

`types` mirrors the core contract: a `review` is the title plus every changed
file's path, old path, status and hunks with their rows (kind, old and new
line numbers, text), and an `annotation` is the M4 anchor, severity, title and
optional body. Annotators declare themselves in the manifest:

```toml
[[annotators]]
id = "todo"
component = "annotators/todo.wasm"
```

and run as `<extension id>/<annotator id>` beside the built-in ones.

Sandboxing and limits:

- World 0.1 imports nothing: no WASI, so no filesystem, network, clock or
  process access. The whole review arrives as data.
- Every call gets a fresh store with a 64 MB memory cap and a two-second
  wall-clock budget enforced by epoch interruption. A call that traps, runs
  out of time or memory, or returns an error yields no annotations, and the
  reason reaches the status line.
- Components compile on first use, off the UI thread. `diffz --doctor`
  compiles every one and reports failures.

`crates/core/tests/fixtures/extensions/todo` is a complete example: a Rust
guest that flags `TODO` on added lines. Rebuild it from `source/` with
`cargo build --release --target wasm32-unknown-unknown`, then
`wasm-tools component new <module>.wasm -o annotators/todo.wasm`.

Deferred:

- **Host imports and capabilities.** Reading whole files, settings, or running
  a linter needs imports and a manifest capability the user grants once per
  extension version.
- **Commands and declarative panels** in the same world: commands export
  `{id, title, default-key}` and a `run` function returning annotations or a
  panel built from list, tree, markdown, table and button nodes.

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
- Whether the binary growth from wasmtime, about 9 MB, is acceptable for users
  who never install an extension, or `wasm` should become opt-in.
