# diffz-core

The UI-free core of [diffz](https://github.com/zzwong/diffz), a desktop tool for
reading and reviewing diffs. It has no GUI or network code; the app and its
adapters supply those.

- `patch`: a parser for unified patches that rejects bad or unsupported input
  instead of guessing.
- `domain`: snapshots, source identity and repository paths, including paths
  that are not valid UTF-8.
- `session` and `review`: deterministic review state, draft comments and the
  outbox that guards submission to GitHub or GitLab.
- `syntax` (feature `syntax`): tree-sitter highlighting.
- `theme` and `palette`: light and dark themes and Omarchy `colors.toml` palettes.

The API exists to serve the diffz app and changes with it. Semver applies, but
expect breaking releases while the version is 0.x.

## Features

- `syntax`: tree-sitter highlighting.
- `gleam`: adds Gleam highlighting. It needs a newer grammar than the crates.io
  release, so it only builds inside the diffz workspace.
- `wasm`: loads grammar extensions as WebAssembly components through wasmtime.
  Building it needs cmake.

## License

MIT
