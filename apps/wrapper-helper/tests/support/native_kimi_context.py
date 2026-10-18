from pathlib import Path
import http.server
import json
import os
import subprocess
import sys
import threading
import tomllib

binary=Path(sys.argv[1]).resolve()
root=Path(sys.argv[2]).resolve()
wrappers=root/'wrappers'
helper=wrappers/('codux-wrapper-helper.exe' if os.name=='nt' else 'codux-wrapper-helper')
home=root/'user home'
kimi_home=root/'kimi home'
project=root/'project with spaces'
for path in (home,kimi_home,project): path.mkdir(parents=True)
requests=[]
class Handler(http.server.BaseHTTPRequestHandler):
    def do_POST(self):
        row=json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        requests.append(row)
        (root/f'request-{len(requests)}.json').write_text(json.dumps(row,ensure_ascii=False,indent=2),encoding='utf-8')
        chunks=[dict(id='owned-fixture',object='chat.completion.chunk',created=0,model='owned-model',choices=[dict(index=0,delta=dict(role='assistant',content='Owned response.'),finish_reason=None)]),dict(id='owned-fixture',object='chat.completion.chunk',created=0,model='owned-model',choices=[dict(index=0,delta={},finish_reason='stop')],usage=dict(prompt_tokens=100,completion_tokens=5,total_tokens=105))]
        data=(''.join('data: '+json.dumps(chunk)+'\n\n' for chunk in chunks)+'data: [DONE]\n\n').encode()
        self.send_response(200);self.send_header('Content-Type','text/event-stream');self.send_header('Content-Length',str(len(data)));self.end_headers();self.wfile.write(data)
    def log_message(self,*args): pass
server=http.server.ThreadingHTTPServer(('127.0.0.1',0),Handler)
threading.Thread(target=server.serve_forever,daemon=True).start()
context=root/'carried rules.md'
rules='Carried native rule ${cwd} `literal` "quoted" 中文\n'*2200
context.write_text(rules,encoding='utf-8')
initial_rules_bytes=len(context.read_bytes())
user_hook=root/'user hook.py'
user_hook.write_text('import json,sys\njson.load(sys.stdin)\nprint(json.dumps({"message":"Original user hook sentinel"}))\n',encoding='utf-8')
user_command=f'"{sys.executable}" "{user_hook}"'
config=f'''# Original user configuration
default_model = "qa" # Keep this comment
telemetry = false
hooks = [{{event="UserPromptSubmit",command={json.dumps(user_command)},timeout=10}}]
[providers.qa]
type = "openai"
api_key = "codux-owned-local-fixture"
base_url = "http://127.0.0.1:{server.server_port}/v1"
[models.qa]
provider = "qa"
model = "owned-model"
max_context_size = 524288
capabilities = []
[watch]
enabled = false
'''
(kimi_home/'config.toml').write_text(config,encoding='utf-8')
(kimi_home/'SYSTEM.md').write_text('Original SYSTEM sentinel.\n${cwd}\n',encoding='utf-8')
profile=root/'kept agent.md'
profile.write_text('---\nname: user-kept\ndescription: Original user profile\ntools: []\n---\nOriginal custom agent sentinel. ${cwd}\n',encoding='utf-8')
(root/'settings.json').write_text('{"kimi":"default","kimiModel":""}',encoding='utf-8')
env=dict(os.environ,HOME=str(home),USERPROFILE=str(home),APPDATA=str(home/'roaming'),LOCALAPPDATA=str(home/'local'),
         KIMI_CODE_HOME=str(kimi_home),PYTHONUTF8='1',DMUX_SESSION_ID='owned-terminal',DMUX_PROJECT_ID='owned-project',
         DMUX_PROJECT_PATH=str(project),DMUX_APP_SUPPORT_ROOT=str(root/'support'),DMUX_AI_RUNTIME_BINDING_DIR=str(root/'bindings'),
         DMUX_TOOL_PERMISSION_SETTINGS_FILE=str(root/'settings.json'),CODUX_BLACKBOARD_CONTEXT_FILE=str(context))
for key in list(env):
    if any(part in key for part in ['API_KEY','TOKEN','CREDENTIAL','ANTHROPIC_','OPENAI_','GOOGLE_','GEMINI_']): env.pop(key,None)
    if key in ['CODUX_NATIVE_KIMI_CONTEXT_MANIFEST','CODUX_MANAGED_INSTRUCTIONS_HELPER','DMUX_EXTERNAL_SESSION_ID','DMUX_ACTIVE_AI_RESOLVED_PATH','CODUX_AGENT_WORKTREE_AUTONOMOUS']: env.pop(key,None)
path=str(binary.parent)+os.pathsep+os.environ.get('PATH',os.environ.get('Path',''))
for key in list(env):
    if key.lower()=='path': env.pop(key)
env['PATH']=path
env['DMUX_ORIGINAL_PATH']=path
entry=root/'invoke.ps1'
entry.write_text('$case=Get-Content -LiteralPath $env:CODUX_TEST_KIMI_CASE -Raw -Encoding UTF8 | ConvertFrom-Json\n& $env:CODUX_TEST_KIMI_WRAPPER $env:CODUX_TEST_KIMI_ALIAS @case\nexit $LASTEXITCODE\n',encoding='utf-8-sig')
env['CODUX_TEST_KIMI_WRAPPER']=str(wrappers/'tool-wrapper.ps1')
env['CODUX_TEST_KIMI_ALIAS']='kimi'
case_number=0
def run(args,direct=False,overrides=None):
    global case_number
    case_number+=1
    current=dict(env,**(overrides or {}))
    if direct:
        launch=root/f'direct-{case_number}.json'
        launch.write_text(json.dumps(dict(tool='kimi',binary=str(binary),args=args,cwd=str(project))),encoding='utf-8')
        command=[str(helper),'--codux-wrapper-helper','native-launch',str(launch)]
    elif os.name=='nt':
        args_file=root/f'args-{case_number}.json'
        args_file.write_text(json.dumps(args,ensure_ascii=False),encoding='utf-8')
        current['CODUX_TEST_KIMI_CASE']=str(args_file)
        command=['powershell.exe','-NoLogo','-NoProfile','-ExecutionPolicy','Bypass','-File',str(entry)]
    else:
        command=[os.environ['CODUX_TEST_ZSH'],'-f','-c','if [[ -n ${CODUX_TEST_ZSH_MODULE_DIR:-} ]]; then module_path=($CODUX_TEST_ZSH_MODULE_DIR $module_path); fi; source "$1" "${@:2}"','codux-test',str(wrappers/'tool-wrapper.sh'),current['CODUX_TEST_KIMI_ALIAS'],*args]
    result=subprocess.run(command,cwd=project,env=current,stdin=subprocess.DEVNULL,capture_output=True,text=True,encoding='utf-8',timeout=45)
    (root/f'stdout-{case_number}.jsonl').write_text(result.stdout,encoding='utf-8')
    (root/f'stderr-{case_number}.txt').write_text(result.stderr,encoding='utf-8')
    return result
def expect_model(args,task,system,rules_body=rules,alias='kimi'):
    before=len(requests)
    result=run(args,overrides={'CODUX_TEST_KIMI_ALIAS':alias})
    assert result.returncode==0,(result.returncode,result.stderr[-1200:],result.stdout[-600:])
    assert len(requests)==before+1,'Expected one actual model request'
    request=requests[-1]
    assert request['model']=='owned-model'
    systems=[message['content'] for message in request['messages'] if message['role']=='system']
    assert len(systems)==1 and system in systems[0],'Original system instructions must remain'
    def message_texts(message):
        content=message['content']
        return [part['text'] for part in content if part.get('type')=='text'] if isinstance(content,list) else [content]
    latest=next(message for message in reversed(request['messages']) if message['role']=='user' and task in message_texts(message))
    content=latest['content']
    texts=[part['text'] for part in content if part.get('type')=='text'] if isinstance(content,list) else [content]
    assert texts[-1]==task,'The original task must survive argv and prompt assembly'
    assert any(rules_body.replace('\r\n','\n').rstrip() in text.replace('\r\n','\n') for text in texts),'All carried rules must reach the model literally'
    assert any('Original user hook sentinel' in text for text in texts),'Existing user hooks must keep running'
    metadata=[json.loads(line) for line in result.stdout.splitlines() if line.startswith('{')]
    session=next(row['session_id'] for row in metadata if row.get('type')=='session.resume_hint')
    return session,systems[0]

try:
    first='Initial task "quoted" ${cwd}\nsecond line'
    default,_=expect_model(['--prompt',first,'--output-format','stream-json'],first,'Original SYSTEM sentinel.')
    task='Task for the unchanged custom agent.'
    session,system=expect_model(['--agent-file',str(profile),'--model','qa','--prompt',task,'--output-format','stream-json'],task,'Original custom agent sentinel.')
    resume='Continue the existing conversation.'
    restored,restored_system=expect_model(['--session',session,'--prompt',resume,'--output-format','stream-json'],resume,'Original custom agent sentinel.')
    assert restored==session and restored_system==system,'Resume must preserve the bound agent'
    changed='Explicitly synced carried policy ${cwd} 中文\n'
    context.write_text(changed,encoding='utf-8')
    next_task='Continue after synchronizing carried rules.'
    continued,_=expect_model(['--continue','--prompt',next_task,'--output-format','stream-json'],next_task,'Original custom agent sentinel.',changed)
    assert continued==session,'--continue must select the existing session'
    manifests=sorted(root.glob('native-kimi-context-*.json'),key=lambda path:path.stat().st_mtime_ns)
    assert manifests
    before=len(requests)
    missing=root/'missing-manifest.json'
    result=run(['--session',session,'--prompt','Do not run when carried rules are missing.','--output-format','stream-json'],direct=True,
               overrides={'CODUX_NATIVE_KIMI_CONTEXT_MANIFEST':str(missing),'CODUX_MANAGED_INSTRUCTIONS_HELPER':str(helper)})
    assert len(requests)==before,'A missing manifest must block the actual model request'
    assert 'Owned response.' not in result.stdout
    context.unlink()
    result=run(['--session',session,'--prompt','Do not run when the carried file is missing.','--output-format','stream-json'],direct=True,
               overrides={'CODUX_NATIVE_KIMI_CONTEXT_MANIFEST':str(manifests[-1]),'CODUX_MANAGED_INSTRUCTIONS_HELPER':str(helper)})
    assert len(requests)==before,'A missing carried file must block the actual model request'
    assert 'Owned response.' not in result.stdout
    outside=run(['--prompt','An ordinary CLI session outside Codux.','--output-format','stream-json'],direct=True)
    assert outside.returncode==0 and len(requests)==before+1
    outside_text=json.dumps(requests[-1],ensure_ascii=False)
    assert 'Carried native rule' not in outside_text and 'Explicitly synced' not in outside_text,'The registered hook must stay inert outside Codux'
    binding=root/'bindings/owned-terminal-kimi.json'
    before_binding=binding.read_bytes()
    before_metadata=len(requests)
    alias=run(['--version'],overrides={'CODUX_TEST_KIMI_ALIAS':'kimi-code'})
    assert alias.returncode==0 and '2.2.0' in alias.stdout,('The declared kimi-code alias must find the primary CLI',alias.stderr[-500:])
    for args in [['provider','list'],['session','list'],['--model','qa','session','list']]:
        metadata=run(args)
        assert metadata.returncode==0,('Metadata commands must not prepare missing project rules',metadata.stderr[-800:])
        assert binding.read_bytes()==before_binding,'Metadata must not become an AI session'
        assert len(requests)==before_metadata
    context.write_text(changed,encoding='utf-8')
    expect_model(['--prompt','provider','--output-format','stream-json'],'provider','Original SYSTEM sentinel.',changed,alias='kimi-code')
    parsed=tomllib.loads((kimi_home/'config.toml').read_text(encoding='utf-8'))
    assert parsed['default_model']=='qa' and parsed['providers']['qa']['api_key']=='codux-owned-local-fixture'
    assert len(parsed['hooks'])==2 and parsed['hooks'][0]['command']==user_command
    assert profile.read_text(encoding='utf-8').startswith('---\nname: user-kept')
    print(json.dumps(dict(ok=True,rules_bytes=initial_rules_bytes,requests=len(requests),fresh_default=True,custom_profile=True,resume=True,continue_session=True,sync=True,missing_manifest_blocks=True,missing_carried_file_blocks=True,outside_hook_inert=True,metadata_passthrough=True,literal_command_word_prompt=True,kimi_code_alias=True,fixture=str(root))))
finally:
    server.shutdown();server.server_close()
