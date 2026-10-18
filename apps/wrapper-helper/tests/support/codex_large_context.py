import base64, http.server, json, os, pathlib, queue, shutil, stat, subprocess, sys, tempfile, threading

binary,helper,wrapper=map(pathlib.Path,sys.argv[1:])
root=pathlib.Path(tempfile.mkdtemp(prefix="codux-large-context-"))
owned=root/"中文 $'` paths"
home=owned/'home'
project=owned/'project'
wrappers=owned/'wrappers'
for path in [home,project/'.codex',wrappers]:path.mkdir(parents=True,exist_ok=True)
shutil.copyfile(helper,wrappers/'codux-wrapper-helper.exe')
shutil.copyfile(wrapper,wrappers/'tool-wrapper.ps1')
(wrappers/'tool-drivers.json').write_text('{"tools":[{"aliases":["codex"],"memoryInjection":"codexDeveloperInstructions"}]}')
settings=owned/'settings.json'
settings.write_text('{"codex":"default","codexModel":""}')
rules="Hard rule 中文 preserve every line\n"*3000
context=owned/'context.md'
context.write_bytes(rules.encode('utf-8'))
project_config='developer_instructions="Trusted project instructions"\nmodel_reasoning_effort="low"\n'
(project/'.codex/config.toml').write_text(project_config)
entry=owned/'entry.ps1'
entry.write_text("""[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
$launch=@('codex')
if($env:TEST_NO_PROFILE -ne '1'){$launch+=@('--profile','work')}
if($env:TEST_CLI_HOOKS){$launch+=@('-c',$env:TEST_CLI_HOOKS)}
if($env:TEST_DISABLE_HOOKS -eq '1'){$launch+=@('--disable','hooks')}
$launch+=@('exec','--skip-git-repo-check','--sandbox','read-only','--json','Complete the local fixture. 中文 "quoted" $() `literal`')
& (Join-Path $env:TEST_ROOT 'wrappers/tool-wrapper.ps1') @launch
exit $LASTEXITCODE
""",encoding='utf-8-sig')
env=dict(os.environ,CODEX_HOME=str(home),TEST_ROOT=str(owned),CODUX_BLACKBOARD_CONTEXT_FILE=str(context),DMUX_TOOL_PERMISSION_SETTINGS_FILE=str(settings),DMUX_SETTINGS_PATH=str(settings),OTEL_SDK_DISABLED='true')
for key in ['OPENAI_API_KEY','CODEX_CHATGPT_AUTH_TOKENS','DMUX_SESSION_ID','DMUX_AI_RUNTIME_BINDING_DIR','DMUX_RUNTIME_EVENT_DIR','DMUX_AI_MEMORY_WORKSPACE_ROOT','CODUX_AGENT_WORKTREE_AUTONOMOUS']:
    env.pop(key,None)
env['PATH']=str(binary.parent)+os.pathsep+env['PATH']
env['DMUX_ORIGINAL_PATH']=env['PATH']
requests=[]
class Server(http.server.BaseHTTPRequestHandler):
    def log_message(self,*args):pass
    def do_POST(self):
        requests.append(json.loads(self.rfile.read(int(self.headers['Content-Length']))))
        item=dict(id='msg_qa',type='message',role='assistant',status='completed',content=[dict(type='output_text',text='Local fixture completed.',annotations=[])])
        events=[dict(type='response.created',response=dict(id='resp_qa',object='response',status='in_progress',output=[])),dict(type='response.output_item.done',output_index=0,item=item),dict(type='response.completed',response=dict(id='resp_qa',object='response',status='completed',output=[item],usage=dict(input_tokens=1,output_tokens=1,total_tokens=2)))]
        payload=''.join('event: '+e['type']+'\ndata: '+json.dumps(e)+'\n\n' for e in events).encode()
        self.send_response(200);self.send_header('Content-Type','text/event-stream');self.send_header('Content-Length',str(len(payload)));self.end_headers();self.wfile.write(payload)
server=http.server.ThreadingHTTPServer(('127.0.0.1',0),Server)
threading.Thread(target=server.serve_forever,daemon=True).start()

def ps_command(script):
    return 'powershell.exe -NoLogo -NoProfile -NonInteractive -EncodedCommand '+base64.b64encode(script.encode('utf-16-le')).decode()
def quote(value):return "'"+str(value).replace("'","''")+"'"
marker=owned/'user-hook.txt'
user_command=ps_command('[IO.File]::WriteAllText('+quote(marker)+',"original user hook")')
group='{hooks=[{type="command",command='+json.dumps(user_command)+',timeout=15}]}'
profile='model="qa-profile-model"\nhooks.SessionStart=['+group+']\n'

def hook_hash():
    # This metadata request never creates a turn or executes a hook.
    launch=owned/'metadata-launch.json'
    launch.write_text(json.dumps(dict(tool='codex',binary=str(binary),args=['--enable','hooks','--disable','plugins','-c','hooks.SessionStart=['+group+']','app-server','--stdio'],cwd=str(project))),encoding='utf-8')
    args=[str(helper),'--codux-wrapper-helper','native-launch',str(launch)]
    process=subprocess.Popen(args,cwd=project,env=env,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.DEVNULL,text=True,encoding='utf-8',creationflags=subprocess.CREATE_NO_WINDOW)
    rows=queue.Queue()
    def read():
        for line in process.stdout:rows.put(json.loads(line))
        rows.put(None)
    threading.Thread(target=read,daemon=True).start()
    def call(id,method,params):
        process.stdin.write(json.dumps(dict(id=id,method=method,params=params))+'\n');process.stdin.flush()
        while True:
            value=rows.get(timeout=15)
            assert value is not None,'metadata exited'
            if value.get('id')==id:
                assert 'error' not in value,value
                return value['result']
    try:
        call(1,'initialize',dict(clientInfo=dict(name='codux_context_qa',version='1'),capabilities=dict(experimentalApi=True)))
        process.stdin.write('{"method":"initialized","params":{}}\n');process.stdin.flush()
        value=call(2,'hooks/list',dict(cwds=[str(project)]))
        return next(h['currentHash'] for h in value['data'][0]['hooks'] if h['source']=='sessionFlags' and h.get('command')==user_command)
    finally:
        process.stdin.close()
        try:process.wait(timeout=3)
        except subprocess.TimeoutExpired:
            subprocess.run(['taskkill.exe','/PID',str(process.pid),'/T','/F'],capture_output=True,creationflags=subprocess.CREATE_NO_WINDOW)
            process.wait(timeout=3)

base='model="qa-root-model"\ndeveloper_instructions="Root user instructions"\nmodel_provider="qa"\n[features]\nplugins=false\nhooks=true\n[model_providers.qa]\nname="Local QA"\nbase_url="http://127.0.0.1:'+str(server.server_address[1])+'/v1"\nwire_api="responses"\nrequires_openai_auth=false\nrequest_max_retries=0\nstream_max_retries=0\n[projects.'+json.dumps(str(project))+']\ntrust_level="trusted"\n'
(home/'config.toml').write_text(base)
(home/'work.config.toml').write_text(profile)
hash_value=hook_hash()
reports=[]
try:
    for has_project,selected,hook_state,cli_hook in [(True,True,'trusted',False),(True,True,'untrusted',False),(True,True,'disabled',False),(False,True,'trusted',False),(True,False,'untrusted',False),(True,True,'untrusted',True)]:
        marker.unlink(missing_ok=True)
        if has_project:(project/'.codex/config.toml').write_text(project_config)
        else:(project/'.codex/config.toml').unlink(missing_ok=True)
        env['TEST_NO_PROFILE']='0' if selected else '1'
        env['TEST_CLI_HOOKS']='hooks.SessionStart=['+group+']' if cli_hook else ''
        config=base
        if hook_state!='untrusted':
            key=str(home/'work.config.toml')+':session_start:0:0'
            config+='\n[hooks.state.'+json.dumps(key)+']\ntrusted_hash='+json.dumps(hash_value)+'\n'
            if hook_state=='disabled':config+='enabled=false\n'
        (home/'config.toml').write_text(config)
        original_config=(home/'config.toml').read_bytes()
        original_profile=(home/'work.config.toml').read_bytes()
        before=len(requests)
        process=subprocess.Popen(['powershell.exe','-NoLogo','-NoProfile','-ExecutionPolicy','Bypass','-File',str(entry)],cwd=project,env=env,stdin=subprocess.DEVNULL,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True,encoding='utf-8',creationflags=subprocess.CREATE_NO_WINDOW)
        try:stdout,stderr=process.communicate(timeout=60)
        except subprocess.TimeoutExpired:
            subprocess.run(['taskkill.exe','/PID',str(process.pid),'/T','/F'],capture_output=True,creationflags=subprocess.CREATE_NO_WINDOW)
            stdout,stderr=process.communicate(timeout=5)
            raise AssertionError('owned CLI timed out: '+stderr[-1500:])
        (root/'diagnostic.json').write_text(json.dumps(dict(fixture=str(root),project=has_project,profile=selected,hookState=hook_state,stdout=stdout,stderr=stderr,requests=requests[before:]),ensure_ascii=False,indent=2),encoding='utf-8')
        assert process.returncode==0,stderr[-4000:]
        assert len(requests)==before+1,'Expected one local model request'
        request=requests[-1]
        texts=[p['text'] for m in request['input'] if m.get('role')=='developer' for p in m.get('content',[]) if p.get('type')=='input_text']
        prefix='Trusted project instructions' if has_project else 'Root user instructions'
        expected=prefix+'\n\n'+rules
        assert any(expected in t for t in texts),'Rules truncated or missing; fixture='+str(root)+'\n'+stderr[-2000:]
        assert sum(prefix in t for t in texts)==1,'Duplicate configured instructions'
        assert request['model']==('qa-profile-model' if selected else 'qa-root-model')
        if has_project:assert request['reasoning']['effort']=='low'
        assert marker.exists()==(selected and hook_state=='trusted'),'Original hook trust changed'
        assert (home/'config.toml').read_bytes()==original_config
        assert (home/'work.config.toml').read_bytes()==original_profile
        if has_project:assert (project/'.codex/config.toml').read_text()==project_config
        assert any('Complete the local fixture. 中文 "quoted" $() `literal`' in p.get('text','') for m in request['input'] if m.get('role')=='user' for p in m.get('content',[]))
        reports.append(dict(project=has_project,profile=selected,originalHookState=hook_state,cliHook=cli_hook,rulesBytes=len(rules.encode()),model=request['model']))
    # Exercise the real hook consumer after a previously prepared snapshot is
    # lost. The helper's JSON alone cannot prove the CLI stops before a turn.
    derived=next(home.glob('codux-startup-*.config.toml'))
    name=derived.name.removesuffix('.config.toml')
    payload=home/(name.replace('codux-startup-','codux-context-')+'.md')
    assert payload.is_file()
    payload.unlink()
    launch=owned/'missing-context-launch.json'
    launch.write_text(json.dumps(dict(tool='codex',binary=str(binary),args=['--profile',name,'--enable','hooks','-c','developer_instructions=""','exec','--skip-git-repo-check','--sandbox','read-only','--json','Stop if project instructions are missing.'],cwd=str(project))),encoding='utf-8')
    before=len(requests)
    missing=subprocess.run([str(helper),'--codux-wrapper-helper','native-launch',str(launch)],cwd=project,env=env,stdin=subprocess.DEVNULL,capture_output=True,text=True,encoding='utf-8',timeout=30,creationflags=subprocess.CREATE_NO_WINDOW)
    (root/'missing-context-diagnostic.json').write_text(json.dumps(dict(code=missing.returncode,stdout=missing.stdout,stderr=missing.stderr,requests=requests[before:]),ensure_ascii=False,indent=2),encoding='utf-8')
    assert len(requests)==before,'Missing startup instructions must not send a model request: '+missing.stdout[-1000:]
    stopped=[json.loads(line) for line in missing.stdout.splitlines() if line.strip()]
    completed=[row for row in stopped if row.get('type')=='turn.completed']
    assert len(completed)==1 and completed[0]['usage']['input_tokens']==0 and completed[0]['usage']['output_tokens']==0,stopped
    assert not any(row.get('item',{}).get('type')=='agent_message' for row in stopped),stopped
    reports.append(dict(missingSnapshot='stopped before model request'))
    env['TEST_CLI_HOOKS']=''
    env['TEST_DISABLE_HOOKS']='1'
    before=len(requests)
    disabled=subprocess.run(['powershell.exe','-NoLogo','-NoProfile','-ExecutionPolicy','Bypass','-File',str(entry)],cwd=project,env=env,stdin=subprocess.DEVNULL,capture_output=True,text=True,encoding='utf-8',timeout=30,creationflags=subprocess.CREATE_NO_WINDOW)
    assert disabled.returncode!=0 and 'startup hooks are disabled' in disabled.stderr
    assert len(requests)==before,'Disabled startup hooks must not send a model request'
finally:
    server.shutdown();server.server_close()
print(json.dumps(dict(ok=True,cases=reports,fixture=str(root)),ensure_ascii=False))
assert root.resolve().parent==pathlib.Path(tempfile.gettempdir()).resolve() and root.name.startswith('codux-large-context-')
def remove_readonly(function,path,error):
    resolved=pathlib.Path(path).resolve()
    assert root.resolve()==resolved or root.resolve() in resolved.parents
    if isinstance(error,PermissionError):
        os.chmod(path,stat.S_IWRITE|stat.S_IREAD)
        function(path)
    else:raise error
shutil.rmtree(root,onexc=remove_readonly)
