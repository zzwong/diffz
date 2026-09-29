#!/usr/bin/env python3
# Local-only transport fixture standing in for gh and glab: replays real compare responses
# recorded from public repositories (tests/fixtures/compare) and never touches a network.
import sys, json, pathlib, urllib.parse
root = pathlib.Path(__file__).parent
fixtures = pathlib.Path('FIXTURES')
args = sys.argv[1:]
# A request body follows the endpoint as `--input -`.
endpoint = args[-3] if args[-2:] == ['--input', '-'] else args[-1]
with (root / 'calls.log').open('a') as log:
    log.write(endpoint + '\n')
host = args[args.index('--hostname') + 1]
want_diff = 'Accept: application/vnd.github.diff' in args
def load(name):
    return json.loads((fixtures / name).read_text())
code = 200; raw = None; value = None
if host == 'github.com':
    gh = load('github_compare.json')
    base, head = gh['base_commit']['sha'], gh['commits'][-1]['sha']
    # Head behind the base, or identical refs: GitHub lists no commits and the merge base is the head.
    if (root / 'behind').exists() or (root / 'identical').exists():
        gh['commits'], gh['total_commits'] = [], 0
        gh['merge_base_commit']['sha'] = head = '1' * 40 if (root / 'behind').exists() else base
    wide = load('github_range.json')
    wide_base, wide_head = wide['base_commit']['sha'], wide['commits'][-1]['sha']
    prefix = 'repos/dtolnay/anyhow'
    if endpoint == 'graphql':
        # Blame queries name each path as a variable $pN, answered under the alias fN.
        body = json.loads(sys.stdin.read())
        blame = load('github_blame.json')
        paths = {k[1:]: v for k, v in body['variables'].items() if k.startswith('p')}
        failing = (root / 'blame-fails').read_text().split() if (root / 'blame-fails').exists() else []
        if (root / 'blame-down').exists():
            code = 502; value = {'message': 'Server Error'}
        else:
            found = {f'f{i}': None if p in failing else {'ranges': blame.get(p, [])} for i, p in paths.items()}
            value = {'data': {'repository': {'object': found}}}
            if any(p in failing for p in paths.values()):
                value['errors'] = [{'message': 'blame timed out'}]
    elif endpoint == prefix:
        value = load('github_repo.json')
    elif endpoint in (f'{prefix}/compare/1.0.80...1.0.81', f'{prefix}/compare/dtolnay%3A1.0.80...fork%3A1.0.81', f'{prefix}/compare/{base}...{head}'):
        if (root / 'diverged').exists():
            gh['merge_base_commit']['sha'] = '0' * 40
        if (root / 'many-commits').exists():
            gh['total_commits'] = 297
        if want_diff and (root / 'no-diff').exists():
            code = 406; value = {'message': 'Sorry, this diff is taking too long to generate.'}
        elif want_diff:
            raw = (fixtures / 'github_compare.diff').read_text()
        else:
            value = gh
    elif endpoint.startswith(f'{prefix}/compare/{base}...{head}?per_page=100&page='):
        first = endpoint.endswith('&page=1')
        value = dict(gh, commits=gh['commits'] if first else [])
    elif endpoint in (f'{prefix}/compare/1.0.78...1.0.81', f'{prefix}/compare/{wide_base}...{wide_head}'):
        if want_diff:
            raw = (fixtures / 'github_range.diff').read_text()
        else:
            value = wide
            # Past 250 commits GitHub lists only the newest; pages of 100 hold them all.
            if (root / 'paged').exists():
                value['commits'] = value['commits'][-10:]
    elif endpoint.startswith(f'{prefix}/compare/{wide_base}...{wide_head}?per_page=100&page='):
        page = int(endpoint.rsplit('=', 1)[1])
        value = dict(wide, commits=wide['commits'][(page - 1) * 100:page * 100])
    elif endpoint.startswith(f'{prefix}/compare/') and endpoint.endswith('?per_page=1'):
        value = load('github_steps.json')[endpoint[len(prefix) + 9:-len('?per_page=1')]]
    elif endpoint == f'{prefix}/tags?per_page=100&page=1':
        if (root / 'no-tags').exists():
            code = 502; value = {'message': 'Server Error'}
        else:
            value = load('github_tags.json')
            # A hotfix tagged on a release branch, off the compared range.
            # A tag on a second-parent commit the range merged, off its first-parent path.
            if (root / 'side-tag').exists():
                value.insert(0, {'name': '1.0.80-hotfix', 'commit': {'sha': 'a2eb7dd5e13add83f254b6dac0f68e043effc521'}})
            # A tag on a blob, as git/git's junio-gpg-pub, has no commit.
            if (root / 'blob-tag').exists():
                value.insert(0, {'name': 'gpg-pub', 'commit': {'sha': ''}})
    elif endpoint == f'{prefix}/releases?per_page=100':
        value = load('github_releases.json')
    else:
        raise AssertionError(endpoint)
else:
    assert host == 'gitlab.com'
    prefix = 'projects/gitlab-org%2Fruby%2Fgems%2Fgitlab-styles'
    frm, to, mb = load('gitlab_from.json'), load('gitlab_to.json'), load('gitlab_merge_base.json')
    wide = load('gitlab_range_from.json')
    if endpoint == prefix:
        value = load('gitlab_project.json')
    elif endpoint in (f'{prefix}/repository/commits/14.0.0', f'{prefix}/repository/commits/{frm["id"]}'):
        value = frm
    elif endpoint in (f'{prefix}/repository/commits/14.1.0', f'{prefix}/repository/commits/{to["id"]}'):
        value = to
    elif endpoint == f'{prefix}/repository/commits/13.1.0':
        value = wide
    elif endpoint == f'{prefix}/repository/merge_base?refs[]={frm["id"]}&refs[]={to["id"]}':
        value = mb
    elif endpoint == f'{prefix}/repository/merge_base?refs[]={wide["id"]}&refs[]={to["id"]}':
        value = load('gitlab_range_merge_base.json')
    elif endpoint.startswith(f'{prefix}/repository/compare?from={frm["id"]}&to={to["id"]}&straight='):
        value = load('gitlab_compare.json')
        if (root / 'timeout').exists():
            value['compare_timeout'] = True
        if (root / 'collapsed').exists():
            value['diffs'][0].update(collapsed=True, diff='')
    elif endpoint.startswith(f'{prefix}/repository/compare?from={wide["id"]}&to={to["id"]}&straight='):
        value = load('gitlab_range.json')
    elif endpoint.startswith(f'{prefix}/repository/compare?from={wide["id"]}&to={frm["id"]}&straight='):
        value = load('gitlab_step.json')
    elif endpoint == f'{prefix}/repository/tags?per_page=100&page=1':
        value = load('gitlab_tags.json')
        if (root / 'blob-tag').exists():
            value.insert(0, {'name': 'gpg-pub', 'commit': None, 'release': None})
    elif endpoint.startswith(f'{prefix}/repository/files/') and '/blame?' in endpoint:
        # Recorded whole; a range is cut from it the way GitLab numbers it.
        path, query = endpoint[len(prefix) + 18:].split('/blame?')
        path = urllib.parse.unquote(path)
        q = dict(urllib.parse.parse_qsl(query))
        assert q['ref'] == to['id'], query
        blame = load('gitlab_blame.json')
        if path not in blame:
            code = 404; value = {'message': '404 File Not Found'}
        else:
            lines = [(g['commit'], l) for g in blame[path] for l in g['lines']]
            value = []
            for commit, line in lines[int(q['range[start]']) - 1:int(q['range[end]'])]:
                if value and value[-1]['commit'] == commit:
                    value[-1]['lines'].append(line)
                else:
                    value.append({'commit': commit, 'lines': [line]})
    else:
        raise AssertionError(endpoint)
sys.stdout.write('HTTP/1.1 %d Mock\r\nContent-Type: application/json\r\n\r\n' % code)
sys.stdout.write(raw if raw is not None else json.dumps(value))
