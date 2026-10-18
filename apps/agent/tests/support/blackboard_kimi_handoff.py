"""Actual hosted Kimi CLI -> runtime transcript monitor -> project blackboard."""
import base64
import http.server
import json
import os
from pathlib import Path
import queue
import shlex
import shutil
import stat
import signal
import subprocess
import sys
import tempfile
import threading
import time
import uuid

binary, agent_binary = map(Path, sys.argv[1:3])
mode = sys.argv[3]
assert mode in ('default', 'custom', 'terminal', 'concurrent')
token = uuid.uuid4().hex
root = Path(tempfile.mkdtemp(prefix='codux-kimi-handoff-'))
print('Owned Kimi handoff fixture: ' + str(root), flush=True)
(root / 'owner.txt').write_text('codux-kimi-handoff-qa:' + token)
home = root / 'home'
kimi_home = home / '.kimi-code' if mode in ('default', 'concurrent') else root / 'relocated kimi home'
project = root / 'project'
for path in (home, kimi_home, project, root / 'temp', root / 'roaming', root / 'local'):
    path.mkdir(parents=True, exist_ok=True)
rules = 'Required carried rule ${cwd} 中文.\n' * 2500
handoff = '## Completed\nHosted Kimi completed this task.\n\n## Handoff\n' + 'Keep the complete result 中文 beyond preview limits.\n' * 100 + 'HANDOFF-END-' + token
requests = []
request_barrier = threading.Barrier(2) if mode == 'concurrent' else None


class Server(http.server.BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def do_POST(self):
        request = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        requests.append(request)
        reply = handoff + '\nSECOND-TERMINAL' if 'secondary handoff task' in json.dumps(request) else handoff
        if request_barrier:
            request_barrier.wait(timeout=15)
        time.sleep(2)
        chunks = [
            dict(id='owned-kimi', object='chat.completion.chunk', created=0, model='owned-model', choices=[dict(index=0, delta=dict(role='assistant', content=reply), finish_reason=None)]),
            dict(id='owned-kimi', object='chat.completion.chunk', created=0, model='owned-model', choices=[dict(index=0, delta={}, finish_reason='stop')], usage=dict(prompt_tokens=100, completion_tokens=5, total_tokens=105)),
        ]
        body = (''.join('data: ' + json.dumps(chunk) + '\n\n' for chunk in chunks) + 'data: [DONE]\n\n').encode()
        self.send_response(200)
        self.send_header('Content-Type', 'text/event-stream')
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)


server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Server)
threading.Thread(target=server.serve_forever, daemon=True).start()
(kimi_home / 'config.toml').write_text(f'''default_model = "qa"
telemetry = false
[providers.qa]
type = "openai"
api_key = "owned-local-fixture"
base_url = "http://127.0.0.1:{server.server_port}/v1"
[models.qa]
provider = "qa"
model = "owned-model"
max_context_size = 524288
capabilities = []
[watch]
enabled = false
''', encoding='utf-8')
profile = root / 'kept agent.md'
profile.write_text('---\nname: owned-user\ndescription: Keep the original agent\ntools: []\n---\nOriginal user agent sentinel.\n', encoding='utf-8')
entry = root / ('entry.ps1' if os.name == 'nt' else 'entry.py')
if os.name == 'nt':
    entry.write_text('''[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
kimi --agent-file $env:CODUX_QA_AGENT --prompt $env:CODUX_QA_TASK --output-format stream-json
$result=@{exit=$LASTEXITCODE;label=$env:CODUX_TERMINAL_LABEL;instance=$env:DMUX_SESSION_INSTANCE_ID;context=$env:CODUX_BLACKBOARD_CONTEXT_FILE;source=(Get-Command kimi).Source}
[IO.File]::WriteAllText($env:CODUX_QA_RESULT,($result|ConvertTo-Json),[Text.UTF8Encoding]::new($false))
''', encoding='utf-8-sig')
    command = "& '" + str(entry).replace("'", "''") + "'"
else:
    entry.write_text('''import json,os,shutil,subprocess
from pathlib import Path
source=shutil.which('kimi')
assert source,'Managed Kimi wrapper is missing'
wrapper=Path(source).parent.parent/'tool-wrapper.sh'
args=['--agent-file',os.environ['CODUX_QA_AGENT'],'--prompt',os.environ['CODUX_QA_TASK'],'--output-format','stream-json']
result=subprocess.run([os.environ['CODUX_TEST_ZSH'],'-f','-c','if [[ -n ${CODUX_TEST_ZSH_MODULE_DIR:-} ]]; then module_path=($CODUX_TEST_ZSH_MODULE_DIR $module_path); fi; source "$1" "${@:2}"','owned-kimi',str(wrapper),'kimi',*args],timeout=45)
report=dict(exit=result.returncode,label=os.environ['CODUX_TERMINAL_LABEL'],instance=os.environ['DMUX_SESSION_INSTANCE_ID'],context=os.environ['CODUX_BLACKBOARD_CONTEXT_FILE'],source=source)
Path(os.environ['CODUX_QA_RESULT']).write_text(json.dumps(report),encoding='utf-8')
''', encoding='utf-8')
    command = shlex.join([sys.executable, str(entry)])
env = dict(os.environ, HOME=str(home), USERPROFILE=str(home), APPDATA=str(root / 'roaming'), LOCALAPPDATA=str(root / 'local'), TEMP=str(root / 'temp'), TMP=str(root / 'temp'))
for key in list(env):
    if any(part in key for part in ['API_KEY', 'TOKEN', 'CREDENTIAL', 'ANTHROPIC_', 'OPENAI_', 'GOOGLE_', 'GEMINI_']):
        env.pop(key, None)
for key in ['KIMI_CODE_HOME', 'KIMI_SHARE_DIR', 'CODUX_AGENT_DATA_DIR', 'DMUX_SESSION_ID', 'DMUX_EXTERNAL_SESSION_ID', 'DMUX_AI_RUNTIME_BINDING_DIR', 'DMUX_RUNTIME_EVENT_DIR', 'DMUX_AI_MEMORY_WORKSPACE_ROOT', 'CODUX_BLACKBOARD_CONTEXT_FILE', 'CODUX_NATIVE_KIMI_CONTEXT_MANIFEST', 'CODUX_MANAGED_INSTRUCTIONS_HELPER', 'CODUX_AGENT_WORKTREE_AUTONOMOUS']:
    env.pop(key, None)
if mode == 'custom':
    env['KIMI_CODE_HOME'] = str(kimi_home)
for key in list(env):
    if key.lower() == 'path':
        env.pop(key)
env['PATH'] = str(binary.parent) + os.pathsep + os.environ.get('PATH', os.environ.get('Path', ''))
stderr = (root / 'agent-stderr.log').open('w', encoding='utf-8')
agent = subprocess.Popen([str(agent_binary), 'runtime-stdio'], cwd=project, env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=stderr, text=True, encoding='utf-8', creationflags=subprocess.CREATE_NO_WINDOW if os.name == 'nt' else 0, start_new_session=os.name != 'nt')
messages = queue.Queue()
events = []


def reader():
    try:
        for line in agent.stdout:
            messages.put(json.loads(line))
    except Exception as error:
        messages.put(dict(readerError=str(error)))
    finally:
        messages.put(None)


threading.Thread(target=reader, daemon=True).start()
request_id = 0


def receive():
    row = messages.get(timeout=30)
    assert row is not None, 'Owned agent exited'
    assert 'readerError' not in row, row
    return row


def rpc(method, **params):
    global request_id
    request_id += 1
    agent.stdin.write(json.dumps(dict(kind='request', id=request_id, method=method, params=params), ensure_ascii=False) + '\n')
    agent.stdin.flush()
    while True:
        row = receive()
        if row.get('id') == request_id:
            assert row['kind'] == 'response', row
            return row['result']
        events.append(row)


project_id = 'owned-kimi-' + token
terminal = 'owned-terminal-' + token
created = None
second_created = None
try:
    hello = receive()
    assert hello['kind'] == 'hello' and hello['platform'] == ('windows' if os.name == 'nt' else 'linux'), hello
    rpc('blackboard.write', projectId=project_id, component='rules', title='Carried rule', body=rules, metadata=dict(enabled=True))
    report = root / 'terminal-result.json'
    terminal_env = dict(CODUX_QA_RESULT=str(report), CODUX_QA_AGENT=str(profile), CODUX_QA_TASK='Complete the isolated Kimi handoff task.')
    if mode == 'terminal':
        terminal_env['KIMI_CODE_HOME'] = str(kimi_home)
    if os.name != 'nt':
        terminal_env.update({key:os.environ[key] for key in ['CODUX_TEST_ZSH','CODUX_TEST_ZSH_MODULE_DIR','LD_LIBRARY_PATH'] if key in os.environ})
    created = rpc('terminal.create', cwd=str(project), shell='powershell.exe' if os.name == 'nt' else '/bin/bash', cols=100, rows=30, rootProjectId=project_id, rootProjectPath=str(project), projectId=project_id, projectName='Kimi handoff QA', terminalId=terminal, sessionKey='owned-kimi-pane', title='Kimi task', runtimeTarget=dict(kind='local'), command=command, env=terminal_env)
    if mode == 'concurrent':
        second_report = root / 'second-result.json'
        second_terminal = terminal + '-second'
        second_env = dict(terminal_env, CODUX_QA_RESULT=str(second_report), CODUX_QA_TASK='Complete the secondary handoff task.')
        second_created = rpc('terminal.create', cwd=str(project), shell='powershell.exe' if os.name == 'nt' else '/bin/bash', cols=100, rows=30, rootProjectId=project_id, rootProjectPath=str(project), projectId=project_id, projectName='Kimi handoff QA', terminalId=second_terminal, sessionKey='owned-kimi-pane-second', title='Second Kimi task', runtimeTarget=dict(kind='local'), command=command, env=second_env)
    deadline = time.monotonic() + 30
    snapshot = None
    while time.monotonic() < deadline:
        snapshot = rpc('blackboard.snapshot', projectId=project_id)
        summaries = [row for row in snapshot['entries'] if row['component'] == 'summary']
        if len(summaries) == (2 if mode == 'concurrent' else 1) and report.is_file() and (mode != 'concurrent' or second_report.is_file()):
            break
        if report.is_file() and json.loads(report.read_text(encoding='utf-8'))['exit'] != 0:
            raise AssertionError('Actual Kimi failed: ' + report.read_text(encoding='utf-8'))
        time.sleep(.2)
    else:
        raise AssertionError('No complete Kimi handoff; fixture=' + str(root))
    result = json.loads(report.read_text(encoding='utf-8'))
    assert result['exit'] == 0 and result['label'].startswith('T01'), result
    assert Path(result['source']).parent.name == 'bin' and Path(result['source']).parent.parent.name == 'wrappers', result
    assert len(requests) == (2 if mode == 'concurrent' else 1), len(requests)
    request = requests[0]
    assert request['model'] == 'owned-model'
    systems = [message['content'] for message in request['messages'] if message['role'] == 'system']
    assert len(systems) == 1 and 'Original user agent sentinel.' in systems[0]
    assert rules.replace('\r\n', '\n').rstrip() in json.dumps(request, ensure_ascii=False).replace('\\n', '\n').replace('\\r', '\r').replace('\r\n', '\n')
    summary = next(row for row in summaries if row['terminalId'] == terminal)
    assert summary['body'] == handoff, (len(summary['body']), len(handoff), summary['body'][-150:])
    metadata = summary['metadata']
    assert metadata['tool'] == 'kimi' and metadata['terminalInstanceId'] == result['instance'], metadata
    assert metadata['sessionId'] and Path(metadata['transcriptPath']).is_relative_to(kimi_home), metadata
    if mode == 'concurrent':
        second_result = json.loads(second_report.read_text(encoding='utf-8'))
        second_summary = next(row for row in summaries if row['terminalId'] == second_terminal)
        assert second_result['exit'] == 0 and second_result['label'].startswith('T02'), second_result
        assert second_summary['body'] == handoff + '\nSECOND-TERMINAL'
        assert second_summary['metadata']['terminalInstanceId'] == second_result['instance']
        assert second_summary['metadata']['sessionId'] != metadata['sessionId']
    time.sleep(1)
    stable = rpc('blackboard.snapshot', projectId=project_id)
    assert [row for row in stable['entries'] if row['component'] == 'summary'] == summaries
    evidence = dict(ok=True, homeMode=mode, terminal=result, summaries=summaries, requests=len(requests), rulesBytes=len(rules.encode()), handoffBytes=len(handoff.encode()))
    (root / 'verification.json').write_text(json.dumps(evidence, ensure_ascii=False, indent=2), encoding='utf-8')
    print(json.dumps({key: evidence[key] for key in ['ok', 'homeMode', 'requests', 'rulesBytes', 'handoffBytes']}))
finally:
    (root / 'captured-requests.json').write_text(json.dumps(requests, ensure_ascii=False), encoding='utf-8')
    (root / 'runtime-events.json').write_text(json.dumps(events, ensure_ascii=False), encoding='utf-8')
    output = b''.join(base64.b64decode(row['params']['bytes']) for row in events if row.get('method') == 'terminal.output')
    (root / 'terminal-output.txt').write_bytes(output)
    for created_terminal in [created, second_created]:
        if not created_terminal:
            continue
        try:
            rpc('terminal.close', sessionId=created_terminal['id'])
        except Exception:
            pass
    agent.stdin.close()
    try:
        agent.wait(timeout=10)
    except subprocess.TimeoutExpired:
        if os.name == 'nt':
            subprocess.run(['taskkill.exe', '/PID', str(agent.pid), '/T', '/F'], capture_output=True, creationflags=subprocess.CREATE_NO_WINDOW)
        else:
            assert os.getpgid(agent.pid) == agent.pid
            os.killpg(agent.pid, signal.SIGKILL)
        agent.wait(timeout=5)
    stderr.close()
    server.shutdown()
    server.server_close()

assert root.resolve().parent == Path(tempfile.gettempdir()).resolve() and root.name.startswith('codux-kimi-handoff-')


def remove_readonly(function, path, error):
    resolved = Path(path).resolve()
    assert root.resolve() == resolved or root.resolve() in resolved.parents
    if isinstance(error, PermissionError):
        os.chmod(path, stat.S_IWRITE | stat.S_IREAD)
        function(path)
    else:
        raise error


shutil.rmtree(root, onexc=remove_readonly)
