#!/usr/bin/env bash
# Experimental upstream PR #4184, ported onto rules_rust 0.74.0.
set -euo pipefail
cd "$(dirname "$0")/.."
export DIFFZ_BAZEL_OUTPUT_ROOT="$PWD/target/bazel-worker"
command="${1:-}"
case "$command" in
    build|test|run|aquery|cquery|fetch) ;;
    clean) exec bash scripts/bazel-trial.sh "$command" --symlink_prefix=target/bazel-worker- "${@:2}" ;;
    *) exec bash scripts/bazel-trial.sh "$@" ;;
esac

patch_file="$PWD/bazel/experimental/persistent-workers.patch"
native_patch="$PWD/bazel/build-script-resources.patch"
source_sha=d8bcc1e111e98270dc03172e22ef7af9c2f94f3269801618a2b83d20890f8df5
fingerprint="$( { printf '%s\n' "$source_sha"; sha256sum "$patch_file" "$native_patch" | cut -d' ' -f1; } | sha256sum | cut -d' ' -f1)"
source_root="$PWD/target/bazel-worker-rules-${fingerprint:0:12}"
if [[ ! -f "$source_root/.diffz-worker-source" ]]; then
    [[ ! -e "$source_root" ]] || {
        echo "Refusing to overwrite existing worker sources: $source_root" >&2
        exit 2
    }
    scratch="$(mktemp -d "${TMPDIR:-/var/tmp}/diffz-bazel-worker.XXXXXX")"
    trap 'rm -rf "$scratch"' EXIT
    curl -fLsS --retry 2 \
        https://github.com/bazelbuild/rules_rust/releases/download/0.74.0/rules_rust-0.74.0.tar.gz \
        -o "$scratch/rules.tar.gz"
    printf '%s  %s\n' "$source_sha" "$scratch/rules.tar.gz" | sha256sum --check --status
    mkdir "$scratch/source"
    tar -xzf "$scratch/rules.tar.gz" -C "$scratch/source"
    patch --batch -p1 -d "$scratch/source" < "$native_patch" >&2
    patch --batch -p1 -d "$scratch/source" < "$patch_file" >&2
    printf '%s\n' "$fingerprint" > "$scratch/source/.diffz-worker-source"
    mkdir -p target
    mv "$scratch/source" "$source_root"
    rm -rf "$scratch"
    trap - EXIT
fi

override="--override_module=rules_rust=$source_root"
if [[ "$command" == fetch ]]; then
    exec bash scripts/bazel-trial.sh "$command" "$override" --lockfile_mode=off "${@:2}"
fi
state="$PWD/target/bazel-worker-state"
mkdir -p "$state"
# Pipelining defaults off: the draft worker currently collides with read-only
# metadata outputs when pipelining is enabled. Keep state on disk in the checkout.
exec bash scripts/bazel-trial.sh "$command" "$override" --lockfile_mode=off \
    --symlink_prefix=target/bazel-worker- \
    --config=native8 --config=cgu256 \
    --@rules_rust//rust/settings:experimental_persistent_worker=true \
    --action_env="TMPDIR=$state" \
    --test_env="TMPDIR=$state" --sandbox_writable_path="$state" \
    --strategy=Rustc=worker,local --strategy=RustcMetadata=worker,local "${@:2}"
