#!/usr/bin/env python3
# Local-only transport fixture: never invokes glab or a network client.
import sys,json,pathlib
root=pathlib.Path(__file__).parent
state_path=root/'fake-state.json'
state=json.loads(state_path.read_text()) if state_path.exists() else {'notes':[], 'discussions':[], 'approved':False}
args=sys.argv[1:]; method=args[args.index('--method')+1]; endpoint=args[args.index('--include')+1].split('?')[0]
assert args[args.index('--hostname')+1]=='gitlab.com'
base='a'*40; head=('c' if (root/'changed-head').exists() else 'b')*40
meta={'author':{'username':'mr-author'},'project_id':9,'iid':7,'title':'Mock merge request','description':'Offline description','state':'opened','diff_refs':{'start_sha':base,'base_sha':base,'head_sha':head}}
position={'position_type':'text','old_path':'a.rs','new_path':'a.rs','old_line':1,'new_line':1,'head_sha':head}
notes=[{'id':1,'body':'Existing thread','author':{'username':'reviewer'},'position':position},{'id':2,'body':'Reply','author':{'username':'me'}}]
discussions=[{'id':'thread','notes':notes},{'id':'general','notes':[{'id':3,'body':'General note','author':{'username':'me'}}]}]+state['discussions']
code=200; raw=False
if method=='POST':
    body=json.load(sys.stdin)
    if endpoint.endswith('/discussions'):
        note={'id':100+len(state['discussions']),'body':body['body'],'position':body['position'],'author':{'username':'me'},'type':'DiffNote'}
        value={'id':'new-discussion','notes':[note]};state['discussions'].append(value);code=201
    elif endpoint.endswith('/approve'):state['approved']=True;value={};code=201
    elif endpoint.endswith('/notes'):
        if (root/'fail-summary').exists():code=503;value={'error':'mock interrupted'}
        else:value={'id':200+len(state['notes']),'body':body['body'],'author':{'username':'me'},'type':None};state['notes'].append(value);code=201
    else:raise AssertionError(endpoint)
    state_path.write_text(json.dumps(state))
    if endpoint.endswith('/notes') and (root/'lost-response').exists():code=503;value={'error':'lost response after write'}
elif endpoint=='user':value={'username':'me'}
elif endpoint.endswith('/raw_diffs'):value='diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1,2 +1,2 @@\n context\n-old\n+new\n';raw=True
elif endpoint.endswith('/diffs'):value=[{'old_path':'a.rs','new_path':'a.rs','too_large':False,'collapsed':False}]
elif endpoint.endswith('/discussions'):value=discussions
elif endpoint.endswith('/pipelines'):value=[{'id':5,'sha':head,'status':'running','web_url':'https://gitlab.com/team/sub/repo/-/pipelines/5'}]
elif endpoint.endswith('/jobs'):value=[{'id':6,'name':'Test','status':'success'}]
elif endpoint.endswith('/approvals'):value={'approved_by':[{'user':{'username':'me'}}] if state['approved'] else []}
elif '/notes/' in endpoint:value=next(n for n in state['notes'] if n['id']==int(endpoint.rsplit('/',1)[1]))
elif endpoint.endswith('/notes'):value=state['notes']
elif endpoint.endswith('/merge_requests/7'):value=meta
else:raise AssertionError(endpoint)
sys.stdout.write('HTTP/1.1 %d Mock\r\nContent-Type: application/json\r\n\r\n'%code)
sys.stdout.write(value if raw else json.dumps(value))
