#!/usr/bin/env python3
"""Run a development command with one checkout-absolute Kache runtime config."""

import argparse
import copy
import hashlib
import json
import math
import os
from pathlib import Path
import re
import stat
import sys
import tempfile
import tomllib


class ConfigError(Exception):
    pass


def absolute_path(value):
    # Match Kache's supported home expansion; it does not expand $VARIABLES.
    if "\0" in value:
        raise ConfigError("Kache paths cannot contain a NUL character.")
    if value == "~":
        value = str(Path.home())
    elif value.startswith("~/"):
        value = str(Path.home() / value[2:])
    path = Path(value)
    # Do not collapse "..": its meaning can depend on a preceding symlink.
    return path if path.is_absolute() else Path.cwd() / path


def string_spans(text):
    """Locate complete string tokens in TOML that has already been validated."""
    index = 0
    while index < len(text):
        if text[index] == "#":
            newline = text.find("\n", index)
            index = len(text) if newline == -1 else newline + 1
        elif text[index] in "\"'":
            start, quote = index, text[index]
            multiline = text.startswith(quote * 3, index)
            index += 3 if multiline else 1
            while index < len(text):
                if quote == '"' and text[index] == "\\":
                    index += 2
                elif text[index] == quote:
                    end = index + 1
                    if multiline:
                        while end < len(text) and text[end] == quote:
                            end += 1
                        if end - index < 3:
                            index = end
                            continue
                    yield start, end
                    index = end
                    break
                else:
                    index += 1
        else:
            index += 1


def equivalent(left, right):
    # TOML permits NaN, whose ordinary equality is false even after a faithful
    # parse. Treat it as unchanged while checking every other value and type.
    if type(left) is not type(right):
        return False
    if isinstance(left, dict):
        return left.keys() == right.keys() and all(equivalent(left[key], right[key]) for key in left)
    if isinstance(left, list):
        return len(left) == len(right) and all(equivalent(a, b) for a, b in zip(left, right))
    return left == right or isinstance(left, float) and math.isnan(left) and math.isnan(right)


def runtime_config(source):
    """Change exactly cache.runtime_dir, preserving all other TOML semantics."""
    text = source.read_text(encoding="utf-8")
    try:
        parsed = tomllib.loads(text)
    except tomllib.TOMLDecodeError as error:
        raise ConfigError(f"Invalid Kache TOML in {source}: {error}") from error
    cache = parsed.get("cache", {})
    if not isinstance(cache, dict):
        raise ConfigError("Kache cache configuration must be a TOML table.")
    value = cache.get("runtime_dir", "target/kache/runtime")
    if not isinstance(value, str) or not value.strip():
        raise ConfigError("Kache config must define a nonempty string cache.runtime_dir.")
    runtime = str(absolute_path(value))
    expected = copy.deepcopy(parsed)
    expected.setdefault("cache", {})["runtime_dir"] = runtime
    replacement = json.dumps(runtime, ensure_ascii=False)

    def candidates():
        if "runtime_dir" in cache:
            # Match the value token, not its key spelling or table layout. This
            # handles quoted/dotted keys, inline tables and multiline strings
            # without changing comments or serializing unrelated policy fields.
            for start, end in string_spans(text):
                if tomllib.loads("value = " + text[start:end])["value"] == value:
                    yield text[:start] + replacement + text[end:]
        else:
            # Older custom policies can omit runtime_dir. Try the three TOML
            # table layouts and accept only the exact semantic addition below.
            yield "cache.runtime_dir = " + replacement + "\n" + text
            for match in re.finditer(r"(?m)^|\Z", text):
                index = match.start()
                newline = "" if index == 0 or text[index - 1] == "\n" else "\n"
                yield text[:index] + newline + "runtime_dir = " + replacement + "\n" + text[index:]
            for match in re.finditer(r"\{", text):
                for suffix in (",", ""):
                    yield text[:match.end()] + " runtime_dir = " + replacement + suffix + text[match.end():]

    for candidate in candidates():
        try:
            matches = equivalent(tomllib.loads(candidate), expected)
        except tomllib.TOMLDecodeError:
            matches = False
        if matches:
            return candidate.encode(), cache.get("ignore_env") is True
    raise ConfigError("Could not set cache.runtime_dir without changing another Kache setting.")


def validate_snapshot(path, expected):
    metadata = path.lstat()
    if (not stat.S_ISREG(metadata.st_mode) or metadata.st_uid != os.geteuid()
            or metadata.st_mode & 0o077):
        raise ConfigError(f"Refusing a non-private, non-owned or non-regular generated Kache config: {path}")
    if path.read_bytes() != expected:
        raise ConfigError(f"Generated Kache config has unexpected contents: {path}")


def prepare_config(source, checkout):
    data, ignore_env = runtime_config(source)
    directory = checkout / "target"
    directory.mkdir(exist_ok=True)
    for name in ("kache", "config"):
        directory = directory / name
        directory.mkdir(mode=0o700, exist_ok=True)
        metadata = directory.lstat()
        if (not stat.S_ISDIR(metadata.st_mode) or metadata.st_uid != os.geteuid()
                or metadata.st_mode & 0o022):
            raise ConfigError(f"Refusing a non-owned, symlinked or publicly writable generated-config directory: {directory}")
    destination = directory / (hashlib.sha256(data).hexdigest() + ".toml")
    # Publish complete, immutable snapshots. Concurrent commands with different
    # source configs cannot replace the file another compiler is about to read.
    with tempfile.NamedTemporaryFile(prefix=".prepare-", dir=directory, delete=False) as stream:
        temporary = Path(stream.name)
        try:
            stream.write(data)
            stream.flush()
            try:
                os.link(temporary, destination)
            except FileExistsError:
                pass
            validate_snapshot(destination, data)
        finally:
            temporary.unlink()
    return destination, ignore_env


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin", required=True)
    parser.add_argument("--config", required=True)
    parser.add_argument("--socket", required=True)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command[1:] if args.command[:1] == ["--"] else args.command
    if not command:
        parser.error("provide a command after --")
    try:
        checkout = Path.cwd()
        config, ignore_env = prepare_config(absolute_path(args.config), checkout)
        env = os.environ | {
            "KACHE_CONFIG": str(config),
            "KACHE_HOST_CONFIG": "",
            "KACHE_SOCKET_PATH": str(absolute_path(args.socket)),
            "RUSTC_WRAPPER": str(absolute_path(args.bin)),
        }
        if not ignore_env and "KACHE_RUNTIME_DIR" in env:
            # Custom policies may allow this higher-priority override. Keep
            # that precedence, but resolve relative paths before Cargo changes cwd.
            env["KACHE_RUNTIME_DIR"] = str(absolute_path(env["KACHE_RUNTIME_DIR"]))
        os.execvpe(command[0], command, env)
    except (OSError, ConfigError) as error:
        print(f"run-kache: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
