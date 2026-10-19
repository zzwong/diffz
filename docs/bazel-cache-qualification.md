# Qualify the Bazel cache on two Linux hosts

`scripts/qualify-bazel-cache.py` turns the same-host cache experiment into a
repeatable producer/consumer acceptance check. It uses the existing
`scripts/bazel-trial.sh` graph and an existing gRPC cache service. It neither
starts a service nor deploys remote execution.

This implements the tooling in [#125](https://github.com/zzwong/diffz/issues/125).
The actual two-host measurements remain an acceptance step in
[#119](https://github.com/zzwong/diffz/issues/119). Python fixtures validate the
driver's controls and evidence handling; they are not Rust or homelab results.

## What a successful run establishes

The producer builds `//:all_build` with a new output root, disk caching disabled,
remote reads disabled, and synchronous remote uploads enabled. It then executes
`//:tests` locally with cached test results disabled and runs the binary's
`--version` and offline `--inspect --fixture F01` checks.

The consumer uses another new output root, disables disk caching and all remote
uploads, and downloads all cached outputs. Qualification requires positive
`remote cache hit` runner counts and **no other build execution**, except
Bazel's internal actions. A partial hit followed by local compilation produces
a failed report with the runner counts retained. The consumer then executes the
tests locally and compares the test target set, selected linker, binary SHA-256,
version, and complete parsed F01 JSON with the producer.

The commands use `--config=fast-dev`, the committed lockfile in error mode, and
the pinned Bazel executable. Neither normal build defaults nor Cargo change.
Build and forced-test timings include their Bazel invocation/setup; they do not
measure a CI queue. Downloads are eager for this qualification. To choose a
developer download policy, use the separate
[lazy/eager benchmarks](bazel-shared-cache.md#real-source-edits-and-download-policy).

## Prerequisites

Use two actual x86_64 Linux machines with the same clean Git commit and the same
immutable native environment. Prefer an image identified by digest. A label such
as `ubuntu-latest` or a mutable image tag does not establish that environment.
Both machines need the existing graph's native development dependencies and
Python 3.10 or newer. Bootstrap the pinned executable with
`scripts/bazel-trial.sh --version`, or set `BAZEL` to that executable. Preflight
does not bootstrap missing tools.

The cache must isolate action-cache entries by instance name. For the existing
**bazel-remote 2.6.3**, verify that the operator started it with
`--enable_ac_key_instance_mangling`. Its default is **false**: supplying
`--remote_instance_name` alone does not partition its action cache.
See the [versioned server documentation](https://github.com/buchgr/bazel-remote/blob/v2.6.3/README.md).
Use appropriate writer/reader credentials. For an existing Bazel credential
helper, pass `--credential-helper /absolute/path/to/helper`; the driver scopes it
to the cache hostname and records only the executable path. An existing
authenticated private tunnel to the service is another option. The wrapper
disables system and home Bazel rc files, so credentials configured only there
will not be used. The qualifier does not expose arbitrary header or TLS flags.
Credentials do not belong in an endpoint URL. The driver accepts a plain
`grpc://host:port` or `grpcs://host:port` endpoint.

`--confirm-instance-isolation` records the operator's verification of that
service configuration. It is required before any build and **does not probe the
server**. Do not supply it merely to bypass a failure. The driver does not alter
a running service or migrate its store. Enabling instance mangling can change
how existing namespaced entries are found; coordinate that separately with the
cache operator or use a dedicated configured service for this qualification.

The action-cache instance is `diffz-qualification/<native fingerprint>`.
It includes no commit SHA or machine ID, so identical environments retain reuse
across source commits and hosts. The fingerprint is additionally supplied through
`--action_env` and `--repo_env`; these stamps reach actions/repository rules that
consume those environments. The server's instance isolation is the partitioning
requirement for the complete cache.

## Run the producer

Choose evidence directories that do not already exist. Keep them under the
checkout's ignored `target/` directory or outside the checkout. Use the same
literal native environment ID on both machines; substitute the actual immutable
image/environment identifier in these examples.

```sh
python3 scripts/qualify-bazel-cache.py preflight \
  --host-label linux-builder-a \
  --native-environment-id 'registry.example/diffz-native@sha256:ACTUAL_DIGEST' \
  --output target/cache-preflight-a

python3 scripts/qualify-bazel-cache.py producer \
  --host-label linux-builder-a \
  --native-environment-id 'registry.example/diffz-native@sha256:ACTUAL_DIGEST' \
  --cache-endpoint grpcs://cache.example.org:9092 \
  --confirm-instance-isolation \
  --output target/cache-producer-a
```

Preflight records a JSON description of the observed environment. The producer
collects its own fresh preflight, so a stale preflight file is never treated as
the current environment. It refuses a dirty source checkout and known ambient
compiler/library overrides. Resolve mismatches by aligning the environment and
rerunning into a new directory.

To reuse a host-local unpacked repository cache, add
`--repo-contents-cache /srv/diffz-repositories` to the producer and consumer.
That directory must be outside the build checkout and evidence directory.
Each host can use its own location; the location is not part of the native
fingerprint. Record whether it was warm when reporting timings. An action-cold
output root can still have a warm repository cache.

## Run the consumer on the second host

Copy the producer's `result.json` to the second host using the normal file
transfer mechanism. Preserve the full producer evidence directory for review.
Do not copy its Bazel output tree into the consumer's new output location.

```sh
python3 scripts/qualify-bazel-cache.py consumer \
  --host-label linux-builder-b \
  --native-environment-id 'registry.example/diffz-native@sha256:ACTUAL_DIGEST' \
  --cache-endpoint grpcs://cache.example.org:9092 \
  --confirm-instance-isolation \
  --producer-report /path/to/producer-result.json \
  --output target/cache-consumer-b

python3 scripts/qualify-bazel-cache.py compare \
  /path/to/producer-result.json target/cache-consumer-b/result.json
```

The consumer rejects different commits, graph hashes, Bazel binaries, or native
fingerprints before contacting the cache. It also rejects the same observed
machine ID and duplicate host labels. Machine IDs can be cloned or generated by
containers: the operator must independently confirm that these are two machines.
Kernel version and the hashed machine ID are reported separately and do not
invalidate an otherwise matching native fingerprint.

## Native input coverage and remaining limits

Preflight records these inputs:

| Input | Evidence |
| --- | --- |
| Declared environment | Operator-supplied immutable environment ID, OS release, architecture |
| Host build tools | Configured CC/CXX and action PATH; resolved compiler, `ar`, diagnostic PATH `ld`, `pkg-config`, CMake, Make and Perl paths, bytes and version output |
| Selected linker | `.bazelrc`'s explicit Rust-sysroot LLD search path and pinned rustc/LLVM archive hashes; after building, actual `ld.lld` and `rust-lld` bytes and LLD version |
| Native runtime | Resolved libgcc, glibc and libstdc++ bytes; compiler target, sysroot and search paths |
| Installed packages | Full available dpkg, RPM, pacman and Homebrew version inventories |
| GUI/native discovery | `pkg-config` results for fontconfig, FreeType, xkbcommon/X11, Wayland, X11 and OpenSSL |
| Extra native inputs | Repeat `--native-file /absolute/path` on both hosts to include additional specific file paths and bytes |
| Source/toolchain graph | Clean commit, Bazel version/bytes, and hashes of the Bazel config, module/lockfiles, Cargo lockfile and wrapper |

This coverage detects concrete mismatches and narrows the environment being
qualified. It does not hash every header, linker script, shared-library closure,
user-level Cargo configuration, or executable discovered by a build script.
Package versions alone do not detect locally modified package files. The
immutable environment requirement remains necessary until those native inputs
are declared in the build graph. Do not weaken the fingerprint to make two
different installations compare equal.

F01 comes from the committed offline fixture, has a content-derived snapshot ID,
and does not populate a current capture timestamp. The qualifier compares its
complete parsed JSON without removing fields. Runtime checks use an explicit
state directory and isolated XDG config/data/cache/state directories under this
run's evidence directory, so they do not open the user's normal diffz database.
They establish headless binary behavior; GUI startup, macOS, Cargo doctests,
release/LTO builds and remote execution require their own checks.

## Evidence, concurrency and failures

Each run retains `preflight.json`, `result.json`, command/exit/timing JSONL, raw
logs, Bazel JSON event streams, profiles, an owned Bazel output root, and runtime
state. A successful consumer also writes `comparison.json`. Exit status 2 and
`status: failed` indicate a rejected or incomplete run; inspect its error and
raw evidence rather than treating partial cache hits as qualification.

Builds can run concurrently with separate output roots. Forced qualification
tests use the stable `/var/tmp/diffz-bazel-qualification-tests` path and an
exclusive host-local lock at the same path with `.lock` appended. This prevents
the wrapper's temporary-root cleanup from racing another qualification test
run. Lock wait is reported separately. The lock file is intentionally retained;
do not unlink it while a qualifier is running. Individual Rust test launchers
still create isolated children inside the temporary root.

The driver stops only the Bazel server associated with its new output root.
It never deletes existing outputs, edits/restores application source, cleans a
shared cache or shuts down another workspace's server. Evidence and build
outputs remain for inspection and explicit cleanup. A failed shutdown is
reported as a failed run, with the output-root identity retained in the command
records.

The BEP parser follows the pinned
[Bazel 8.4.2 schema](https://github.com/bazelbuild/bazel/blob/8.4.2/src/main/java/com/google/devtools/build/lib/buildeventstream/proto/build_event_stream.proto):
successful final exit codes, action-summary runner counts, executed test results
and test summaries. Missing metrics or unexpected execution kinds fail closed.
JSON path conversion is disabled so BEP retains `file://` artifact locations;
the driver will execute only the downloaded `diffz` binary inside its owned
output root. See the pinned
[BEP transport options](https://github.com/bazelbuild/bazel/blob/8.4.2/src/main/java/com/google/devtools/build/lib/buildeventstream/transports/BuildEventStreamOptions.java).

Run the local policy/driver fixtures with:

```sh
python3 -m unittest discover -s scripts/tests -p test_qualify_bazel_cache.py -v
```

After the two-host run, attach the reports, raw BEP/logs, actual service
configuration evidence, hardware/environment identities, repository-cache
warmth and repeat-run timings to #119. Compare the results with a persistent
optimized Cargo runner before deciding whether an execution cluster is useful.
