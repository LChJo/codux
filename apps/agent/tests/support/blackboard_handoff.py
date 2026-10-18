import http.server,json,os,pathlib,queue,shutil,stat,subprocess,sys,tempfile,threading,time,uuid

binary,agent_binary=map(pathlib.Path,sys.argv[1:])
token=uuid.uuid4().hex
root=pathlib.Path(tempfile.mkdtemp(prefix='codux-handoff-'))
print('Owned handoff fixture: '+str(root),flush=True)
(root/'owner.txt').write_text('codux-handoff-qa:'+token)
home=root/'home'
codex_home=home/'.codex'
project=root/'project'
temp=root/'temp'
for path in [codex_home,project/'.codex',temp,root/'roaming',root/'local']:path.mkdir(parents=True)
rules='Preserve this project rule 中文.\n'*3000
handoff='## Completed\nCodux-hosted CLI completed this task.\n\n## Handoff\n'+('Keep complete handoff 中文 beyond preview limits.\n'*100)+'HANDOFF-END-'+token
requests=[]
class Server(http.server.BaseHTTPRequestHandler):
    def log_message(self,*args):pass
    def do_POST(self):
        requests.append(json.loads(self.rfile.read(int(self.headers['Content-Length']))))
        # Keep a real active turn visible to the runtime's transcript monitor.
        time.sleep(2)
        item=dict(id='msg_qa',type='message',role='assistant',status='completed',content=[dict(type='output_text',text=handoff,annotations=[])])
        events=[dict(type='response.created',response=dict(id='resp_qa',object='response',status='in_progress',output=[])),dict(type='response.output_item.done',output_index=0,item=item),dict(type='response.completed',response=dict(id='resp_qa',object='response',status='completed',output=[item],usage=dict(input_tokens=1,output_tokens=1,total_tokens=2)))]
        body=''.join('event: '+row['type']+'\ndata: '+json.dumps(row)+'\n\n' for row in events).encode()
        self.send_response(200);self.send_header('Content-Type','text/event-stream');self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
server=http.server.ThreadingHTTPServer(('127.0.0.1',0),Server)
threading.Thread(target=server.serve_forever,daemon=True).start()
config='model="qa-handoff-model"\nmodel_provider="qa"\n[features]\nplugins=false\nhooks=true\n[model_providers.qa]\nname="Local QA"\nbase_url="http://127.0.0.1:'+str(server.server_address[1])+'/v1"\nwire_api="responses"\nrequires_openai_auth=false\nrequest_max_retries=0\nstream_max_retries=0\n[projects.'+json.dumps(str(project))+']\ntrust_level="trusted"\n'
(codex_home/'config.toml').write_text(config,encoding='utf-8')
(codex_home/'work.config.toml').write_text('model="qa-selected-profile"\n',encoding='utf-8')
(project/'.codex/config.toml').write_text('developer_instructions="Original project instructions"\n',encoding='utf-8')
entry=root/'entry.ps1'
entry.write_text("""[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
codex --profile work exec --skip-git-repo-check --sandbox read-only --json 'Complete the isolated handoff fixture.'
$result=@{exit=$LASTEXITCODE;label=$env:CODUX_TERMINAL_LABEL;instance=$env:DMUX_SESSION_INSTANCE_ID;context=$env:CODUX_BLACKBOARD_CONTEXT_FILE;source=(Get-Command codex).Source;events=$env:DMUX_RUNTIME_EVENT_DIR;bindings=$env:DMUX_AI_RUNTIME_BINDING_DIR;claudeMap=$env:DMUX_CLAUDE_SESSION_MAP_DIR;opencodeMap=$env:DMUX_OPENCODE_SESSION_MAP_DIR}
[IO.File]::WriteAllText($env:CODUX_QA_RESULT,($result|ConvertTo-Json),[Text.UTF8Encoding]::new($false))
""",encoding='utf-8-sig')
env=dict(os.environ,HOME=str(home),USERPROFILE=str(home),APPDATA=str(root/'roaming'),LOCALAPPDATA=str(root/'local'),TEMP=str(temp),TMP=str(temp),CODEX_HOME=str(codex_home),OTEL_SDK_DISABLED='true')
env['PATH']=str(binary.parent)+os.pathsep+env['PATH']
for key in ['OPENAI_API_KEY','CODEX_CHATGPT_AUTH_TOKENS','CODUX_AGENT_DATA_DIR','DMUX_SESSION_ID','DMUX_AI_RUNTIME_BINDING_DIR','DMUX_RUNTIME_EVENT_DIR','DMUX_AI_MEMORY_WORKSPACE_ROOT','CODUX_BLACKBOARD_CONTEXT_FILE','CODUX_AGENT_WORKTREE_AUTONOMOUS']:
    env.pop(key,None)
stderr=(root/'agent-stderr.log').open('w',encoding='utf-8')
agent=subprocess.Popen([str(agent_binary),'runtime-stdio'],cwd=project,env=env,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=stderr,text=True,encoding='utf-8',creationflags=subprocess.CREATE_NO_WINDOW)
messages=queue.Queue()
events=[]
def reader():
    try:
        for line in agent.stdout:messages.put(json.loads(line))
    except Exception as error:messages.put(dict(readerError=str(error)))
    finally:messages.put(None)
threading.Thread(target=reader,daemon=True).start()
request_id=0
def receive(timeout=30):
    row=messages.get(timeout=timeout)
    assert row is not None,'owned agent exited'
    assert 'readerError' not in row,row
    return row
def rpc(method,**params):
    global request_id
    request_id+=1
    agent.stdin.write(json.dumps(dict(kind='request',id=request_id,method=method,params=params),ensure_ascii=False)+'\n');agent.stdin.flush()
    while True:
        row=receive()
        if row.get('id')==request_id:
            assert row['kind']=='response',row
            return row['result']
        events.append(row)
project_id='qa-handoff-'+token
terminal='qa-codex-'+token
created=None
try:
    hello=receive();assert hello['kind']=='hello' and hello['platform']=='windows',hello
    rule=rpc('blackboard.write',projectId=project_id,component='rules',title='Required rule',body=rules,metadata=dict(enabled=True))
    report=root/'terminal-result.json'
    created=rpc('terminal.create',cwd=str(project),shell='powershell.exe',cols=100,rows=30,rootProjectId=project_id,rootProjectPath=str(project),projectId=project_id,projectName='Handoff QA',terminalId=terminal,sessionKey='qa-handoff-pane',title='Handoff task',runtimeTarget=dict(kind='local'),command="& '"+str(entry).replace("'","''")+"'",env=dict(CODUX_QA_RESULT=str(report)))
    deadline=time.monotonic()+75
    snapshot=None
    while time.monotonic()<deadline:
        snapshot=rpc('blackboard.snapshot',projectId=project_id)
        summaries=[row for row in snapshot['entries'] if row['component']=='summary']
        if summaries and report.is_file():break
        time.sleep(.2)
    else:raise AssertionError('No complete handoff; fixture='+str(root))
    result=json.loads(report.read_text(encoding='utf-8'))
    assert result['exit']==0,result
    assert result['label'].startswith('T01'),result
    assert 'scripts\\wrappers\\bin' in result['source'],result
    owner_temp=home/'.codux-wsl/runtime-temp'
    for field,child in [('events','runtime-events'),('bindings','ai-runtime-bindings'),('claudeMap','claude-session-map'),('opencodeMap','opencode-session-map')]:
        assert pathlib.Path(result[field])==owner_temp/child,result
    assert len(requests)==1,len(requests)
    texts=[part['text'] for row in requests[0]['input'] if row.get('role')=='developer' for part in row.get('content',[]) if part.get('type')=='input_text']
    assert any(rules in text for text in texts),'Rules missing in actual PTY model request'
    assert sum('Original project instructions' in text for text in texts)==1
    assert requests[0]['model']=='qa-selected-profile'
    assert len(summaries)==1 and summaries[0]['terminalId']==terminal,summaries
    assert summaries[0]['body']==handoff,(len(summaries[0]['body']),len(handoff),summaries[0]['body'][-200:])
    assert summaries[0]['metadata']['tool']=='codex'
    assert summaries[0]['metadata']['terminalInstanceId']==result['instance']
    assert summaries[0]['metadata']['sessionId']
    saved_cursor=snapshot['cursor']
    time.sleep(1)
    stable=rpc('blackboard.snapshot',projectId=project_id)
    assert len([row for row in stable['entries'] if row['component']=='summary'])==1
    evidence=dict(ok=True,fixture=str(root),hello=hello,terminal=result,summary=summaries[0],requests=len(requests),rulesBytes=len(rules.encode()),handoffBytes=len(handoff.encode()),cursor=saved_cursor)
    (root/'verification.json').write_text(json.dumps(evidence,ensure_ascii=False,indent=2),encoding='utf-8')
    print(json.dumps(dict(ok=True,fixture=str(root),rulesBytes=evidence['rulesBytes'],handoffBytes=evidence['handoffBytes'])))
finally:
    (root/'captured-requests.json').write_text(json.dumps(requests,ensure_ascii=False),encoding='utf-8')
    (root/'runtime-events.json').write_text(json.dumps(events,ensure_ascii=False),encoding='utf-8')
    if created:
        try:rpc('terminal.close',sessionId=created['id'])
        except Exception:pass
    agent.stdin.close()
    try:agent.wait(timeout=10)
    except subprocess.TimeoutExpired:
        subprocess.run(['taskkill.exe','/PID',str(agent.pid),'/T','/F'],capture_output=True,creationflags=subprocess.CREATE_NO_WINDOW)
        agent.wait(timeout=5)
    stderr.close();server.shutdown();server.server_close()
assert root.resolve().parent==pathlib.Path(tempfile.gettempdir()).resolve() and root.name.startswith('codux-handoff-')
def remove_readonly(function,path,error):
    resolved=pathlib.Path(path).resolve()
    assert root.resolve()==resolved or root.resolve() in resolved.parents
    if isinstance(error,PermissionError):
        os.chmod(path,stat.S_IWRITE|stat.S_IREAD);function(path)
    else:raise error
shutil.rmtree(root,onexc=remove_readonly)
