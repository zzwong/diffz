#!/usr/bin/env python3
"""Offline GitHub compare transport for a base merged as head's second parent.

A -> v1 -> merge (head)
  -> base --^
"""
import json
import sys

A, BASE, V1, MERGE = (c * 40 for c in 'abcd')
repo = 'repos/example/project'
endpoint = sys.argv[-1]
feature = {'filename': 'feature.txt', 'status': 'added', 'additions': 1,
           'deletions': 0, 'changes': 1, 'patch': '@@ -0,0 +1 @@\n+feature'}
base_file = {'filename': 'base.txt', 'status': 'added', 'additions': 1,
             'deletions': 0, 'changes': 1, 'patch': '@@ -0,0 +1 @@\n+base'}
v1_commit = {'sha': V1, 'parents': [{'sha': A}], 'commit': {'message': 'v1'}}
merge_commit = {'sha': MERGE, 'parents': [{'sha': V1}, {'sha': BASE}],
                'commit': {'message': 'merge base'}}

def compare(base, merge_base, commits, files):
    return {'base_commit': {'sha': base}, 'merge_base_commit': {'sha': merge_base},
            'total_commits': len(commits), 'commits': commits, 'files': files}

parent = compare(BASE, BASE, [v1_commit, merge_commit], [feature])
first = compare(BASE, A, [v1_commit], [feature])
tail = compare(V1, V1, [merge_commit], [base_file])
if endpoint == repo:
    value = {'id': 1, 'name': 'project', 'owner': {'login': 'example'}}
elif endpoint == f'{repo}/tags?per_page=100&page=1':
    value = [{'name': 'v1', 'commit': {'sha': V1}}]
elif endpoint == f'{repo}/releases?per_page=100':
    value = []
elif endpoint in (f'{repo}/compare/base...head', f'{repo}/compare/{BASE}...{MERGE}'):
    value = parent
elif endpoint in (f'{repo}/compare/base...v1', f'{repo}/compare/{BASE}...{V1}',
                  f'{repo}/compare/{BASE}...{V1}?per_page=1'):
    value = first
elif endpoint == f'{repo}/compare/{V1}...{MERGE}?per_page=1':
    value = tail
else:
    raise AssertionError(endpoint)
# Use GitHub's documented no-unified-diff response to exercise file-based patch construction.
if 'Accept: application/vnd.github.diff' in sys.argv:
    sys.stdout.write('HTTP/1.1 406 Mock\r\n\r\n')
else:
    sys.stdout.write('HTTP/1.1 200 Mock\r\nContent-Type: application/json\r\n\r\n')
    sys.stdout.write(json.dumps(value))
