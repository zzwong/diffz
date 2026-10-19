#!/usr/bin/env python3
"""Install CI's pinned Rust toolchain without hashing unrelated image toolchains.

Swatinem/rust-cache hashes every toolchain in RUSTUP_HOME, not only the active
compiler. Keep this home at a stable path: RUSTUP_HOME itself is also hashed.
Only this job's new directory is written; the runner's shared Rust home is left
alone. See docs/ci-rust-cache.md for the measured cache miss and limitations.
"""

from __future__ import annotations

import os
from pathlib import Path
import shutil
import subprocess
import sys
import tomllib


class SetupError(RuntimeError):
    pass


def env_path(name: str, env: dict[str, str]) -> Path:
    value = env.get(name, "")
    if not value or "\n" in value or "\r" in value or not Path(value).is_absolute():
        raise SetupError(f"{name} must be an absolute path without line breaks")
    return Path(value)


def text_list(config: dict, name: str) -> list[str]:
    values = config.get(name, [])
    if not isinstance(values, list) or any(
        not isinstance(value, str) or not value.strip() for value in values
    ):
        raise SetupError(f"toolchain.{name} must be a list of nonempty strings")
    return values


def prepare(repo: Path, env: dict[str, str]) -> Path:
    """Prepare one fresh hosted CI job; publish its environment only on success."""
    for name in ("RUSTUP_TOOLCHAIN", "RUSTC"):
        if env.get(name):
            raise SetupError(f"unset {name}; CI must use rust-toolchain.toml")

    runner_temp = env_path("RUNNER_TEMP", env).resolve(strict=True)
    if not runner_temp.is_dir():
        raise SetupError("RUNNER_TEMP must be an existing directory")
    env_file = env_path("GITHUB_ENV", env)
    if env_file.is_symlink() or not env_file.is_file():
        raise SetupError("GITHUB_ENV must be an existing regular file")
    env_file = env_file.resolve(strict=True)
    if not env_file.is_relative_to(runner_temp):
        raise SetupError("GITHUB_ENV must be inside RUNNER_TEMP")

    with (repo / "rust-toolchain.toml").open("rb") as handle:
        config = tomllib.load(handle).get("toolchain", {})
    if not isinstance(config, dict):
        raise SetupError("rust-toolchain.toml must contain a toolchain table")
    channel = config.get("channel")
    if not isinstance(channel, str) or not channel.strip() or "path" in config:
        raise SetupError("rust-toolchain.toml must select a distribution channel")
    profile = config.get("profile", "default")
    if profile not in ("minimal", "default", "complete"):
        raise SetupError("rust-toolchain.toml has an unsupported profile")
    components = text_list(config, "components")
    targets = text_list(config, "targets")
    rustup = shutil.which("rustup", path=env.get("PATH"))
    if not rustup:
        raise SetupError("rustup is required on the hosted runner")

    # A random suffix would change rust-cache's environment hash on every run.
    # Refuse to reuse or delete a pre-existing home, including dangling symlinks.
    rustup_home = runner_temp / "diffz-rustup"
    try:
        rustup_home.mkdir(mode=0o700)
    except FileExistsError as error:
        raise SetupError(f"refusing existing Rust home: {rustup_home}") from error

    child_env = dict(env, RUSTUP_HOME=str(rustup_home))

    def run(*args: str, capture: bool = False) -> str:
        result = subprocess.run(
            args, cwd=repo, env=child_env, check=True, text=True,
            stdout=subprocess.PIPE if capture else None,
        )
        return result.stdout.strip() if capture else ""

    install = [rustup, "toolchain", "install", "--no-self-update", f"--profile={profile}"]
    install += [f"--component={value}" for value in components]
    install += [f"--target={value}" for value in targets]
    run(*install, "--", channel)

    installed = run(rustup, "toolchain", "list", "--quiet", capture=True).splitlines()
    if len(installed) != 1:
        raise SetupError(f"expected one installed toolchain, found {len(installed)}")
    active = run(rustup, "show", "active-toolchain", capture=True)
    if not active or active.split()[0] != installed[0].split()[0]:
        raise SetupError("the repository's active toolchain differs from the installed toolchain")

    compiler = Path(run(rustup, "which", "rustc", capture=True)).resolve(strict=True)
    if not compiler.is_relative_to(rustup_home) or not os.access(compiler, os.X_OK):
        raise SetupError("the selected rustc must be executable inside the job's Rust home")
    expected = run(str(compiler), "-vV", capture=True)
    observed = run("rustc", "-vV", capture=True)
    if expected != observed:
        raise SetupError("rustc on PATH does not select the repository's pinned toolchain")
    print(f"CI Rust home: {rustup_home}\n{active}\n{observed}", flush=True)

    # Following host steps, including rust-cache's post step, use this same home.
    # Failed setup leaves its owned directory for inspection but exports nothing.
    with env_file.open("a", encoding="utf-8") as handle:
        handle.write(f"\nRUSTUP_HOME={rustup_home}\n")
    return rustup_home


def main() -> int:
    try:
        prepare(Path(__file__).resolve().parents[1], dict(os.environ))
    except (OSError, ValueError, SetupError, subprocess.CalledProcessError) as error:
        print(f"CI Rust setup failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
