#!/usr/bin/env python3
"""Asserts the byte-preserving curl boundary without making any request."""
import json
from pathlib import Path
import sys
from urllib.parse import urlsplit, parse_qs, unquote

root = Path(__file__).resolve().parent
args = sys.argv[1:]
config = sys.stdin.read()
assert args[0] == "--disable", "curlrc was not disabled first"
assert "--location" not in args and "-L" not in args, "redirects were enabled"
assert args[args.index("--max-redirs") + 1] == "0"
assert args[args.index("--proto") + 1] == "=https"
assert args[args.index("--config") + 1] == "-"
assert args[args.index("--request") + 1] == "GET"
assert args[args.index("--write-out") + 1] == "\nDIFFZ_ENTIRE_HTTP:%{http_code}"
assert config.startswith('header = "Authorization: Bearer ')
assert config.endswith('"\n') and config.count("\n") == 1
assert not any("Bearer" in a or "signature" in a for a in args), "credential leaked into argv"
url = urlsplit(args[args.index("--url") + 1])
assert url.scheme == "https" and url.netloc == "aws-eu-west-1.api.entire.io"
assert url.path.startswith("/api/v1/git/repos/01JBBBBBBBBBBBBBBBBBBBBBBB/raw/")
assert parse_qs(url.query) in ({"ref": ["b" * 40]}, {"ref": ["c" * 40]})
with (root / "curl-calls.jsonl").open("a") as out:
    out.write(json.dumps({"args": args, "header_in_stdin": True}) + "\n")

scenario_path = root / "scenario"
scenario = scenario_path.read_text().strip() if scenario_path.exists() else "normal"
if scenario == "source-redirect":
    sys.stdout.buffer.write(b"redirect\nDIFFZ_ENTIRE_HTTP:302")
elif scenario == "source-404":
    sys.stdout.buffer.write(b"{\"title\":\"Not Found\"}\nDIFFZ_ENTIRE_HTTP:404")
elif unquote(url.path).endswith("/odd name?#.json"):
    sys.stdout.buffer.write(b'{"exact":"special path"}\nDIFFZ_ENTIRE_HTTP:200')
else:
    sys.stdout.buffer.write(b'{"value":2,"nested":{"x":3}}\nDIFFZ_ENTIRE_HTTP:200')
