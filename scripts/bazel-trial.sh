#!/usr/bin/env bash
set -euo pipefail
script_path="${BASH_SOURCE[0]}"
if [[ "$script_path" != */* ]]; then
    script_path="$(command -v "$script_path")"
fi
while [[ -L "$script_path" ]]; do
    script_dir="$(cd -P "$(dirname "$script_path")" && pwd)"
    script_target="$(readlink "$script_path")"
    if [[ "$script_target" == /* ]]; then
        script_path="$script_target"
    else
        script_path="$script_dir/$script_target"
    fi
done
cd -P "$(dirname "$script_path")/.."
# An absolute output root also avoids relative paths in Crate Universe's mirrors.
if [[ -n "${BAZEL:-}" ]]; then
    bazel_bin="$BAZEL"
else
    [[ "$(uname -s)-$(uname -m)" == Linux-x86_64 ]] || {
        echo 'This Bazel trial currently supports x86_64 Linux.' >&2
        exit 2
    }
    bazel_bin="$PWD/target/bazel-bootstrap/bazel-8.4.2-linux-x86_64"
    if [[ ! -x "$bazel_bin" ]]; then
        bootstrap="$(mktemp -d "${TMPDIR:-/var/tmp}/diffz-bazel-bootstrap.XXXXXX")"
        trap 'rm -rf "$bootstrap"' EXIT
        curl -fLsS --retry 2 \
            https://github.com/bazelbuild/bazel/releases/download/8.4.2/bazel-8.4.2-linux-x86_64 \
            -o "$bootstrap/bazel"
        printf '%s  %s\n' \
            4dc8e99dfa802e252dac176d08201fd15c542ae78c448c8a89974b6f387c282c \
            "$bootstrap/bazel" | sha256sum --check --status
        chmod +x "$bootstrap/bazel"
        mkdir -p "$(dirname "$bazel_bin")"
        mv "$bootstrap/bazel" "$bazel_bin"
        rm -rf "$bootstrap"
        trap - EXIT
    fi
fi
# The worker experiment uses a separate server/artifact root to avoid discarding
# the normal trial's analysis and compiled outputs when switching configurations.
output_root="${DIFFZ_BAZEL_OUTPUT_ROOT:-$PWD/target/bazel}"
# Bazel handles --version before its startup-option parser.
if [[ "${1:-}" == --version ]]; then
    exec "$bazel_bin" "$@"
fi
# Qualification can own a complete process group instead of leaving a detached
# server after its client is interrupted. Everyday trials keep server reuse.
startup_args=()
case "${DIFFZ_BAZEL_BATCH:-0}" in
    0|"") ;;
    1) startup_args+=(--batch) ;;
    *)
        echo 'DIFFZ_BAZEL_BATCH must be 0 or 1.' >&2
        exit 2
        ;;
esac
# Everyday action caching is opt-in. Add cache flags before user arguments so
# explicit CLI flags can override them, including the cold benchmark's blanks.
cache_command="${1:-}"
if [[ "$cache_command" == build || "$cache_command" == test || "$cache_command" == fetch ]]; then
    cache_enabled="${DIFFZ_BAZEL_CACHE:-0}"
    repo_contents_cache="${DIFFZ_BAZEL_REPO_CONTENTS_CACHE:-}"
    cache_args=()

    case "$cache_enabled" in
        1) ;;
        0|"")
            ;;
        *)
            echo 'DIFFZ_BAZEL_CACHE must be 0 or 1.' >&2
            exit 2
            ;;
    esac

    if [[ -n "$repo_contents_cache" ]]; then
        if [[ "$cache_enabled" != 1 ]]; then
            echo 'DIFFZ_BAZEL_REPO_CONTENTS_CACHE requires DIFFZ_BAZEL_CACHE=1.' >&2
            exit 2
        fi
        if [[ "${BAZEL_RUST_STDLIB_LINKFLAGS+x}" ]]; then
            echo 'BAZEL_RUST_STDLIB_LINKFLAGS is an untracked rules_rust input; unset it before enabling the repository contents cache.' >&2
            exit 2
        fi
        if ! repo_contents_cache="$(python3 - "$repo_contents_cache" "$PWD" <<'PY'
import os
import sys

cache = os.path.realpath(os.path.abspath(sys.argv[1]))
workspace = os.path.realpath(sys.argv[2])
try:
    inside_workspace = os.path.commonpath((workspace, cache)) == workspace
except ValueError:
    inside_workspace = False
if inside_workspace:
    print("DIFFZ_BAZEL_REPO_CONTENTS_CACHE must resolve outside the current workspace.", file=sys.stderr)
    raise SystemExit(2)
print(cache)
PY
)"; then
            exit 2
        fi
        cache_args+=("--repo_contents_cache=$repo_contents_cache")
    fi

    if [[ "$cache_command" == build || "$cache_command" == test ]]; then
        if [[ "$cache_enabled" == 1 ]]; then
            cache_args+=("--disk_cache=$PWD/target/bazel-disk-cache")
            cache_args+=(--experimental_disk_cache_gc_max_size=10G)
        fi

        remote_cache="${DIFFZ_BAZEL_REMOTE_CACHE:-}"
        remote_read_only="${DIFFZ_BAZEL_REMOTE_CACHE_READ_ONLY:-0}"
        case "$remote_read_only" in
            0|"")
                ;;
            1)
                if [[ -z "$remote_cache" ]]; then
                    echo 'DIFFZ_BAZEL_REMOTE_CACHE_READ_ONLY=1 requires DIFFZ_BAZEL_REMOTE_CACHE.' >&2
                    exit 2
                fi
                ;;
            *)
                echo 'DIFFZ_BAZEL_REMOTE_CACHE_READ_ONLY must be 0 or 1.' >&2
                exit 2
                ;;
        esac
        if [[ -n "$remote_cache" ]]; then
            if [[ "$cache_enabled" != 1 ]]; then
                echo 'DIFFZ_BAZEL_REMOTE_CACHE requires DIFFZ_BAZEL_CACHE=1.' >&2
                exit 2
            fi
            cache_args+=(--remote_cache="$remote_cache")
            if [[ "$remote_read_only" == 1 ]]; then
                cache_args+=(--remote_upload_local_results=false)
            fi
        fi
    fi
    if ((${#cache_args[@]} > 0)); then
        set -- "$1" "${cache_args[@]}" "${@:2}"
    fi
fi
# Keep the inherited test environment identical for build and test analysis.
export DIFFZ_TEST_TMP_ROOT="${DIFFZ_BAZEL_TEST_TMP_ROOT:-${TMPDIR:-/var/tmp}/diffz-bazel-tests}"
if [[ "${1:-}" == test ]]; then
    # Unix socket tests need a short path; Bazel's default TEST_TMPDIR is too long.
    test_root="$DIFFZ_TEST_TMP_ROOT"
    mkdir -p "$test_root"
    trap 'rmdir "$test_root" 2>/dev/null || true' EXIT
    "$bazel_bin" "${startup_args[@]}" --nosystem_rc --nohome_rc --output_user_root="$output_root" test \
        --sandbox_writable_path="$test_root" "${@:2}"
else
    exec "$bazel_bin" "${startup_args[@]}" --nosystem_rc --nohome_rc --output_user_root="$output_root" "$@"
fi
