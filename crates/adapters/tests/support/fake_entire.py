#!/usr/bin/env python3
"""Recorded-shape Entire v0.12.2 CLI boundary; no network and no credentials.

The JSON shapes follow the published Entire Core/regional OpenAPI. Scenario
files live beside this executable so tests need no shared process environment.
"""
import base64
import json
import os
from pathlib import Path
import sys
import time
from urllib.parse import parse_qs, urlsplit

ROOT = Path(__file__).resolve().parent
scenario_path = ROOT / "scenario"
SCENARIO = scenario_path.read_text().strip() if scenario_path.exists() else "normal"
REPO = "01JBBBBBBBBBBBBBBBBBBBBBBB"
PROJECT = "01JCCCCCCCCCCCCCCCCCCCCCCC"
TRAIL = "01JDDDDDDDDDDDDDDDDDDDDDDD"
CHANGE = "01JEEEEEEEEEEEEEEEEEEEEEEE"
DECOY = "01JFFFFFFFFFFFFFFFFFFFFFFF"
GROUP = "repo_01JGGGGGGGGGGGGGGGGGGGGGGG"
BASE, HEAD, MERGE = "a" * 40, "b" * 40, "c" * 40
API = "https://aws-eu-west-1.api.entire.io"
PROJECT_API = "https://aws-us-east-2.api.entire.io"
FORGE = "gh" if SCENARIO.startswith("gh-") else "et"
PROVIDER = "github" if FORGE == "gh" else "entire"
ROUTING_SCENARIO = SCENARIO[3:] if SCENARIO.startswith("gh-") else SCENARIO


def token(audience="https://us.auth.entire.io", subject="account_1"):
    def enc(value):
        return base64.urlsafe_b64encode(json.dumps(value).encode()).decode().rstrip("=")

    return enc({"alg": "none"}) + "." + enc({"aud": audience, "sub": subject}) + ".signature"


def record(args):
    # Deliberately never record token values; the test can still prove pinning.
    with (ROOT / "calls.jsonl").open("a") as out:
        out.write(json.dumps({
            "args": args,
            "api_origin": os.environ.get("ENTIRE_API_BASE_URL"),
            "has_token": bool(os.environ.get("ENTIRE_TOKEN")),
            "web_override": os.environ.get("ENTIRE_WEB_BASE_URL"),
        }) + "\n")


def die(message):
    print(message, file=sys.stderr)
    sys.exit(90)


def response(value, status=200):
    if SCENARIO != "missing-status":
        print("HTTP/1.1 " + str(status) + " fixture", file=sys.stderr)
    if isinstance(value, bytes):
        sys.stdout.buffer.write(value)
    else:
        # Actual entire api reformats JSON; raw files must never use this path.
        print(json.dumps(value, indent=2))
    sys.exit(0 if status == 200 else 1)


def nth(label):
    path = ROOT / (label + ".count")
    n = int(path.read_text()) + 1 if path.exists() else 1
    path.write_text(str(n))
    return n


args = sys.argv[1:]
record(args)
if args == ["auth", "token"]:
    count = nth("login")
    if SCENARIO == "bad-token":
        print("secret\"\nurl = https://evil.example/")
    elif SCENARIO == "foreign-token":
        print(token("https://evil.example"))
    elif SCENARIO == "core-audience":
        print(token("https://core.eu.entire.io"))
    elif SCENARIO == "nested-core-audience":
        print(token("https://core.eu.unknown.entire.io"))
    else:
        print(token(subject="account_2" if SCENARIO == "account-switch" and count > 1 else "account_1"))
    sys.exit(0)

# Be explicit about the whole invocation rather than accepting arbitrary flags
# that could introduce a write or route around the origin checks.
if len(args) != 7 or args[:2] != ["api", "--to"] or args[3:6] != ["--include", "--method", "GET"]:
    die("unexpected Entire invocation")

backend = args[2]
parsed = urlsplit(args[-1])
path, query = parsed.path, parse_qs(parsed.query)
if parsed.scheme or parsed.netloc or not path.startswith("/api/v1/"):
    die("endpoint is not fixed origin-relative API")
if not os.environ.get("ENTIRE_TOKEN"):
    die("operation did not pin its login")
if backend == "core":
    if os.environ.get("ENTIRE_API_BASE_URL") is not None:
        die("core read inherited data routing override")
elif backend == "cell":
    expected = PROJECT_API if path.startswith("/api/v1/" + FORGE + "/acme/trails/") else API
    if os.environ.get("ENTIRE_API_BASE_URL") != expected:
        die("request went to home cell instead of authoritative primary")
else:
    die("unexpected backend")

if SCENARIO == "unauthorized":
    response({"detail": "untrusted token " + os.environ["ENTIRE_TOKEN"]}, status=401)

if path == "/api/v1/me":
    claims = os.environ["ENTIRE_TOKEN"].split(".")[1]
    claims += "=" * (-len(claims) % 4)
    sub = json.loads(base64.urlsafe_b64decode(claims))["sub"]
    response({"global": {"accountId": sub}, "mode": "global"})

if path == "/api/v1/repos":
    if query != {"filter": [FORGE + "/acme/widget"]}:
        die("repository lookup did not preserve forge namespace")
    count = nth("directory")
    origin = "https://aws-eu-west-1.api.entire.io.evil.example" if SCENARIO == "foreign-placement" else API
    placements = [
        {"id": DECOY, "status": "ready", "apiBaseUrl": PROJECT_API},
        {"id": REPO, "status": "ready", "apiBaseUrl": origin},
    ]
    if SCENARIO == "primary-missing":
        placements.pop()
    if SCENARIO == "primary-not-ready":
        placements[-1]["status"] = "processing"
    if ROUTING_SCENARIO.startswith("catalog"):
        placements[-1].update(clusterSlug="eu-slug", cell="aws-eu-west-1")
        if ROUTING_SCENARIO == "catalog-invalid-inline":
            placements[-1]["apiBaseUrl"] = "https://aws-eu-west-1.api.entire.io.evil.example"
        elif ROUTING_SCENARIO == "catalog-null-inline":
            placements[-1]["apiBaseUrl"] = None
        else:
            del placements[-1]["apiBaseUrl"]
        if ROUTING_SCENARIO.startswith("catalog-inline-to-catalog") and count == 1:
            placements[-1]["apiBaseUrl"] = API
        if ROUTING_SCENARIO.startswith("catalog-to-inline") and count > 1:
            placements[-1]["apiBaseUrl"] = PROJECT_API if ROUTING_SCENARIO.endswith("-switch") else API
        if ROUTING_SCENARIO == "catalog-cell":
            placements[-1]["clusterSlug"] = "not-in-catalog"
        if ROUTING_SCENARIO == "catalog-empty-slug":
            placements[-1]["clusterSlug"] = ""
        if ROUTING_SCENARIO == "catalog-empty-cell":
            placements[-1]["cell"] = ""
        if ROUTING_SCENARIO == "catalog-slug-cell-conflict":
            placements[-1]["cell"] = "aws-us-east-2"
        if ROUTING_SCENARIO == "catalog-reordered":
            placements.reverse()
    row = {"id": GROUP, "full_name": "acme/widget", "provider": PROVIDER,
           "primaries": {"processing": REPO}, "placements": placements}
    if SCENARIO in ("qualified-directory", "gh-qualified", "gh-conflict"):
        row["full_name"] = FORGE + "/acme/widget"
        del row["provider"]
    if SCENARIO == "gh-conflict":
        row["provider"] = "entire"
    if SCENARIO == "gh-unqualified-missing-provider":
        del row["provider"]
    if SCENARIO == "gh-mismatched-qualified":
        row["full_name"] = "et/acme/widget"
    if SCENARIO == "repository-switch" and count > 1:
        row["id"] = "repo_01JHHHHHHHHHHHHHHHHHHHHHHH"
    response({"repos": [
        {"id": "repo_decoy", "full_name": "acme/widget", "provider": "entire" if FORGE == "gh" else "github",
         "primaries": {"processing": DECOY}, "placements": placements}, row,
    ], "truncated": False, "candidatesIncomplete": False})

if path == "/api/v1/clusters":
    if backend != "core" or query or not ROUTING_SCENARIO.startswith("catalog"):
        die("unexpected cluster catalog request")
    count = nth("catalog")
    if ROUTING_SCENARIO == "catalog-denied":
        response({"detail": "do not expose untrusted catalog errors"}, 403)
    if ROUTING_SCENARIO == "catalog-wait":
        (ROOT / "catalog-waiting").write_text("ready")
        time.sleep(2)
    if ROUTING_SCENARIO == "catalog-bytes":
        response(b" " * (1024 * 1024) + b'{"clusters":[]}')
    if ROUTING_SCENARIO == "catalog-malformed":
        response({"clusters": "not an array"})
    clusters = [
        {"slug": "us-slug", "jurisdiction": "us", "isDefault": True,
         "apiUrl": PROJECT_API, "publicUrl": "https://aws-us-east-2.entire.io"},
        {"slug": "eu-slug", "jurisdiction": "eu", "isDefault": False,
         "apiUrl": API, "publicUrl": "https://aws-eu-west-1.entire.io"},
    ]
    if ROUTING_SCENARIO == "catalog-missing":
        clusters.pop()
    elif ROUTING_SCENARIO == "catalog-ambiguous":
        clusters.append(dict(clusters[-1]))
    elif ROUTING_SCENARIO == "catalog-normalized-ambiguous":
        clusters.append(dict(clusters[-1], slug=" EU-SLUG "))
    elif ROUTING_SCENARIO == "catalog-missing-api":
        del clusters[-1]["apiUrl"]
    elif ROUTING_SCENARIO == "catalog-invalid-origin":
        clusters[-1]["apiUrl"] = "https://aws-eu-west-1.api.entire.io.evil.example"
    elif ROUTING_SCENARIO == "catalog-switch" and count > 1:
        clusters[-1]["apiUrl"] = PROJECT_API
    elif ROUTING_SCENARIO == "catalog-inline-to-catalog-switch":
        clusters[-1]["apiUrl"] = PROJECT_API
    elif ROUTING_SCENARIO == "catalog-too-many":
        clusters.extend(dict(clusters[0], slug="extra-" + str(i)) for i in range(1023))
    elif ROUTING_SCENARIO == "catalog-reordered":
        clusters.reverse()
    response({"clusters": clusters})

if path == "/api/v1/projects/resolve/" + FORGE + "/acme":
    if SCENARIO == "repo-only":
        response({"detail": "project resolve is unavailable to this repository reader"}, status=403)
    response({"reference": {"host": FORGE, "project": "acme"},
              "project": {"id": PROJECT, "apiUrl": PROJECT_API,
                          "primaryProcessingCell": "us-cell", "region": "us"}})

if path == "/api/v1/" + FORGE + "/acme/trails/42":
    if query != {"project_id": [PROJECT]}:
        die("project identity was not bound")
    change = {"id": CHANGE, "repositoryId": REPO, "repository": "widget", "number": 7,
              "branch": "feature/work", "base": "main", "status": "open"}
    count = nth("project-detail")
    if SCENARIO == "choice-replaced" and count > 1:
        change["id"] = "different_change"
    if SCENARIO == "choice-placement-switch" and count > 1:
        change["repositoryId"] = DECOY
    changes = [] if SCENARIO == "choice-removed" and count > 1 else [change]
    if SCENARIO == "choice-final-duplicate" and count > 2:
        changes.append(dict(change, repository="other", number=9))
    if SCENARIO == "multiple":
        changes.append(dict(change, id="01JKKKKKKKKKKKKKKKKKKKKKKK", repository="other", number=9))
    trail = {"id": TRAIL, "projectId": PROJECT, "number": 42, "title": "Project intent",
             "body": "Intent", "changes": changes}
    if SCENARIO == "choice-project-switch" and count > 1:
        trail["id"] = DECOY
    if SCENARIO != "omitted-partial-flag":
        trail["isPossiblyPartial"] = SCENARIO == "partial"
    if SCENARIO == "malformed-partial-flag":
        trail["isPossiblyPartial"] = "false"
    response(trail)

def discussion_summary(i):
    return {"id": "discussion_" + str(i), "trail_id": DECOY if SCENARIO == "discussion-cross" else CHANGE,
            "title": "Discussion " + str(i), "kind": "general" if i == 1 else "review_comment",
            "resolved": i == 2, "message_count": 1, "created_at": "2026-11-08T00:00:00Z",
            "updated_at": "2026-11-08T00:01:00Z", "etag": "summary-etag",
            "last_message_at": "2026-11-08T00:01:00Z", "last_message_author": "agent",
            "participants": [{"login": "alice", "identity": {"kind": "user"}, "actor_account_id": "account_1"}], "created_by": "alice", "created_by_identity": {"kind": "user"},
            "resolved_by": None, "resolved_by_identity": None, "resolved_at": None,
            "review_comment_id": None if i == 1 else "finding_2", "review_comment": None if i == 1 else {"code_version_id": "version_old", "location": {"path": "old.txt", "line": 9}}}

if path == "/api/v1/repos/" + REPO + "/trails/7/discussions":
    if SCENARIO == "discussion-denied": response({"detail": "denied"}, 403)
    assert query.get("per_page") == ["100"]
    assert "include_messages" not in query
    page = int(query.get("cursor", ["0"])[0])
    if SCENARIO == "discussion-byte-padding": response(b" " * (8 * 1024 * 1024) + b'{"items":null,"event_cursor":"5"}')
    if SCENARIO == "discussion-byte-cumulative":
        response(b" " * (5 * 1024 * 1024) + json.dumps({"items": [discussion_summary(page + 1)], "event_cursor": "5", "next_cursor": str(page + 1)}).encode())
    if SCENARIO == "discussion-byte-escapes":
        response(b'{"items":null,"event_cursor":"5","unknown":"' + b'\\u0020' * 1400000 + b'"}')
    if SCENARIO == "discussion-malformed": response({"items": "bad", "event_cursor": "5"})
    if SCENARIO == "discussion-empty": response({"items": None, "event_cursor": "5"})
    if SCENARIO == "discussion-overflow": response({"items": [discussion_summary(i) for i in range(101)], "event_cursor": "5"})
    if SCENARIO == "discussion-bad-cursor": response({"items": [], "event_cursor": "5", "next_cursor": "bad\nvalue"})
    if SCENARIO == "discussion-cap":
        response({"items": [discussion_summary(page + 1)], "next_cursor": str(page + 1), "event_cursor": "5"})
    i = 1 if page == 0 or SCENARIO == "discussion-duplicate" else 2
    response({"items": [discussion_summary(i)], "next_cursor": "1" if page == 0 or SCENARIO == "discussion-loop" else "", "event_cursor": str(5 + page)})

if path == "/api/v1/repos/" + REPO + "/trails/7/discussions/discussion_1":
    if SCENARIO == "discussion-detail-byte-padding": response(b" " * (8 * 1024 * 1024) + b'{}')
    reply = {"id": "reply_1", "author": "agent", "identity": {"kind": "agent", "run_id": "run_1"}, "body": "", "created_at": "2026-11-08T00:01:00Z"}
    message = {"id": "message_1", "etag": "message-etag", "author": "alice", "identity": {"kind": "user"}, "body": "Original body", "created_at": "2026-11-08T00:00:00Z", "replies": [reply]}
    if SCENARIO == "discussion-duplicate-message": reply["id"] = "message_1"
    if SCENARIO == "discussion-missing-body": del message["body"]
    if SCENARIO == "discussion-large-body": message["body"] = "x" * 65537
    if SCENARIO == "discussion-too-many-messages": message["replies"] = [dict(reply, id="reply_" + str(i)) for i in range(1001)]
    response({"discussion": discussion_summary(2 if SCENARIO == "discussion-wrong-id" else 1), "messages": [message], "event_cursor": "6"})

if path == "/api/v1/repos/" + REPO + "/trails/7":
    count = nth("detail")
    head = "d" * 40 if (SCENARIO == "moving" or SCENARIO == "discussion-moving") and count % 2 == 0 else HEAD
    base = "e" * 40 if SCENARIO == "moving-base" and count % 2 == 0 else BASE
    parent = {"id": TRAIL, "project_id": PROJECT, "host": FORGE, "project": "acme", "number": 42,
              "path": "/api/v1/" + FORGE + "/acme/trails/" + TRAIL,
              "jurisdiction": "us", "primary_processing_cell": "us-cell"}
    if SCENARIO == "wrong-parent":
        parent["host"] = "wrong-host"
    if SCENARIO == "gh-mixed-parent-case":
        parent["project"] = "Acme"
    if SCENARIO == "wrong-parent-id":
        parent["project_id"] = "01JNNNNNNNNNNNNNNNNNNNNNNN"
    if SCENARIO == "wrong-parent-number":
        parent["number"] = 43
    if SCENARIO == "wrong-parent-path":
        parent["path"] = "/api/v1/et/other/trails/" + TRAIL
    if SCENARIO == "moving-parent" and count % 2 == 0:
        parent["id"] = "01JNNNNNNNNNNNNNNNNNNNNNNN"
        parent["path"] = "/api/v1/" + FORGE + "/acme/trails/" + parent["id"]
    if SCENARIO == "missing-parent":
        parent = None
    response({"id": CHANGE, "number": 7, "base_sha": base,
              "head_sha": None if SCENARIO == "missing-pin" else head,
              "merge_base_sha": MERGE, "base": "main", "branch": None if SCENARIO == "merged" else "feature/work",
              "title": "Capture reasoning alongside code", "parent": parent,
              "status": "merged" if SCENARIO == "merged" else "mystery" if SCENARIO == "unknown-status" else "open",
              "ready_for_review": True, "body_document": {"text_snapshot": "Describe intent"},
              "author": {"login": "reviewer"}, "mergeability": None})

if path == "/api/v1/repos/" + REPO + "/sessions":
    if query != {"base": [MERGE], "head": [HEAD], "limit": ["10"]}:
        die("session metadata was not scoped to the immutable comparison range")
    if SCENARIO == "sessions-denied":
        response({"detail": "session metadata is unavailable"}, status=403)
    if SCENARIO == "sessions-absent":
        response({"detail": "session route not indexed yet"}, status=404)
    if SCENARIO == "sessions-wait":
        (ROOT / "sessions-waiting").write_text("ready")
        time.sleep(2)
    session_scenario = SCENARIO[3:] if SCENARIO.startswith("gh-") else SCENARIO
    session_repo = {
        "sessions-wrong-owner": "other/widget",
        "sessions-wrong-repository": "acme/other",
        "sessions-wrong-empty": "acme/other",
        "sessions-wrong-placement": DECOY,
        "sessions-lowercase-placement": REPO.lower(),
        "sessions-repository-case": "ACME/Widget",
        "sessions-repository-placement": REPO,
    }.get(session_scenario, "acme/widget")
    response({"sessions": [] if session_scenario == "sessions-wrong-empty" else [{"sessionId": "session-one", "agent": "Claude Code",
                            "model": "claude-sonnet", "startedAt": "2026-10-26T12:00:00Z",
                            "lastActivityAt": "2026-10-26T12:30:00Z",
                            "prompt": "PRIVATE SESSION PROMPT MUST NOT PERSIST",
                            "displayName": "PRIVATE DISPLAY NAME MUST NOT PERSIST"}],
              "next_cursor": "opaque-next-page" if SCENARIO == "sessions-partial" else None,
              "repo_full_name": session_repo, "app_has_access": True,
              "session_branch_exists": True, "history_status": "complete",
              "history_complete": True, "source": "db"})

if path == "/api/v1/git/repos/" + REPO + "/compare":
    if query != {"base": [MERGE], "head": [HEAD], "includeStats": ["true"]}:
        die("comparison used mutable refs or target tip instead of frozen merge base")
    files = [
        {"path": "data.json", "status": "modified"},
        {"path": "new.txt", "previousPath": "old.txt", "status": "renamed"},
        {"path": "image.bin", "status": "modified", "binary": True},
        {"path": "empty.txt", "status": "added"},
        {"path": "run.sh", "status": "modified"},
    ]
    if SCENARIO == "large":
        files[0]["tooLarge"] = True
    if SCENARIO == "coverage":
        files.append({"path": "omitted.txt", "status": "modified"})
    response({"files": None if SCENARIO == "null-files" else files, "truncated": SCENARIO == "truncated"})

if path == "/api/v1/git/repos/" + REPO + "/diff":
    if query != {"base": [MERGE], "head": [HEAD]}:
        die("diff did not use immutable comparison pins")
    if SCENARIO in ("empty-diff", "null-files"):
        response(b"")
    response(b'''diff --git a/data.json b/data.json
index 1111111..2222222 100644
--- a/data.json
+++ b/data.json
@@ -1 +1 @@
-{"value":1}
\\ No newline at end of file
+{"value":2,"nested":{"x":3}}
\\ No newline at end of file
diff --git a/old.txt b/new.txt
similarity index 75%
rename from old.txt
rename to new.txt
--- a/old.txt
+++ b/new.txt
@@ -1 +1 @@
-old
+new
diff --git a/image.bin b/image.bin
index 3333333..4444444 100644
Binary files a/image.bin and b/image.bin differ
diff --git a/empty.txt b/empty.txt
new file mode 100644
index 0000000..e69de29
diff --git a/run.sh b/run.sh
old mode 100644
new mode 100755
''')

die("unexpected fixture endpoint: " + path)
