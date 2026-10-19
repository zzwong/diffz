# Incremental Rust worker experiment

This is an opt-in trial of [rules_rust PR #4184](https://github.com/bazelbuild/rules_rust/pull/4184),
upstream commit `9dfe42a282847d3188b404d9f1081ddf403477b7`, ported onto
rules_rust 0.74.0. The port adapts constructor arguments and preserves 0.74.0's
opaque `Args` handling. The worker/protocol implementation is upstream's draft;
it is not part of the released rules used by the normal trial.

```sh
scripts/bazel-worker-trial.sh build //:all_build
scripts/bazel-worker-trial.sh test //:tests
scripts/bazel-worker-trial.sh test //:tests --cache_test_results=no
scripts/bazel-worker-trial.sh shutdown
```

The driver verifies the official rules archive, applies the checked-in patches,
and stores generated sources, build output, and worker state under `target/`.
It uses a separate Bazel output root and convenience symlinks, and leaves the
normal module lockfile alone. Worker mode selects 256 workspace codegen units
and eight AWS-LC native jobs with a 16-CPU scheduler budget.

Incremental state belongs to live worker processes. A server restart may require
seeding it again. Pipelining stays off: combining the draft worker with pipelining
failed when rustc tried to overwrite a read-only metadata artifact. The worker
trial does not establish support for remote execution or shared incremental state.

For a full comparison with normal Cargo, including source restoration and server
shutdown:

```sh
python3 scripts/benchmark-bazel.py \
  --bazel-driver scripts/bazel-worker-trial.sh --iterations 3 \
  --output "$HOME/.local/state/diffz/evidence/bazel-worker-comparison"
```

See [the follow-up measurements](../../docs/bazel-tuning.md) for results and
limitations. Cargo remains the default build and CI workflow.
