import http.server
import json
import os
import pathlib
import queue
import signal
import subprocess
import tempfile
import threading
import time

BINARY = os.environ['BUZZ_TEST_CODEX']
if not pathlib.Path(BINARY).is_absolute():
    raise RuntimeError('BUZZ_TEST_CODEX must be an absolute executable path')
ROOT = pathlib.Path(tempfile.mkdtemp(prefix='buzz-codex-envelope-', dir='/private/tmp'))
for global_config in ['/etc/codex/config.toml', '/etc/codex/requirements.toml', '/Library/Preferences/com.openai.codex.plist']:
    if pathlib.Path(global_config).exists():
        raise RuntimeError('Global configuration exists; refusing synthetic test')
HOME = ROOT / 'home'
PROJECT = ROOT / 'project'
HOME.mkdir()
PROJECT.mkdir()
(PROJECT / 'allowed.txt').write_text('synthetic-inside')
(ROOT / 'outside.txt').write_text('synthetic-outside')
(PROJECT / 'escape.txt').symlink_to(ROOT / 'outside.txt')
CAPTURE = []
SHELL_PROBE = os.environ.get('BUZZ_SYNTHETIC_SHELL_PROBE') == '1'


class Fixture(http.server.BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def do_GET(self):
        self.send_response(404)
        self.end_headers()

    def do_POST(self):
        length = int(self.headers.get('Content-Length', '0'))
        if length > 2000000 or len(CAPTURE) >= 4:
            self.send_error(413)
            return
        raw = self.rfile.read(length)
        request = json.loads(raw)
        CAPTURE.append(request)
        message = {'id': 'msg-fixture', 'type': 'message', 'role': 'assistant', 'status': 'completed', 'content': [{'type': 'output_text', 'text': 'synthetic complete', 'annotations': []}]}
        events = [
            {'type': 'response.created', 'response': {'id': 'resp-fixture', 'status': 'in_progress', 'output': []}},
            {'type': 'response.output_item.added', 'output_index': 0, 'item': dict(message, status='in_progress', content=[])},
            {'type': 'response.output_text.delta', 'item_id': 'msg-fixture', 'output_index': 0, 'content_index': 0, 'delta': 'synthetic complete'},
            {'type': 'response.output_item.done', 'output_index': 0, 'item': message},
            {'type': 'response.completed', 'response': {'id': 'resp-fixture', 'status': 'completed', 'output': [message], 'usage': {'input_tokens': 1, 'output_tokens': 1, 'total_tokens': 2}}},
        ]
        if SHELL_PROBE and len(CAPTURE) == 1:
            tool = next(tool for tool in request['tools'] if tool.get('name') in ('shell', 'shell_command', 'exec_command'))
            command = f'printf "inside="; /bin/cat {PROJECT}/allowed.txt; printf "\\noutside="; /bin/cat {ROOT}/outside.txt; printf "\\nsymlink="; /bin/cat {PROJECT}/escape.txt; printf "\\nwrite="; printf bad > {PROJECT}/model-forbidden.txt'
            if tool['name'] == 'shell':
                arguments = {'command': ['/bin/sh', '-c', command], 'workdir': str(PROJECT), 'timeout_ms': 3000}
            elif tool['name'] == 'exec_command':
                arguments = {'cmd': command, 'workdir': str(PROJECT), 'max_output_tokens': 1000}
            else:
                arguments = {'command': command, 'workdir': str(PROJECT), 'timeout_ms': 3000}
            call = {'id': 'fc-fixture', 'type': 'function_call', 'call_id': 'call-fixture', 'name': tool['name'], 'arguments': json.dumps(arguments)}
            events = [{'type': 'response.output_item.done', 'output_index': 0, 'item': call}, {'type': 'response.completed', 'response': {'id': 'resp-fixture', 'status': 'completed', 'output': [call], 'usage': {'input_tokens': 1, 'output_tokens': 1, 'total_tokens': 2}}}]
        data = ''.join('event: ' + event['type'] + '\ndata: ' + json.dumps(event) + '\n\n' for event in events).encode()
        self.send_response(200)
        self.send_header('Content-Type', 'text/event-stream')
        self.send_header('Content-Length', str(len(data)))
        self.end_headers()
        self.wfile.write(data)


server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Fixture)
threading.Thread(target=server.serve_forever, daemon=True).start()
port = server.server_port
config = f'''model = "fixture"
model_provider = "fixture"
cli_auth_credentials_store = "file"
check_for_update_on_startup = false
default_permissions = "buzz"
approval_policy = "on-request"
approvals_reviewer = "user"
web_search = "disabled"
[features]
shell_tool = {str(SHELL_PROBE).lower()}
unified_exec = false
plugins = false
remote_plugin = false
apps = false
hooks = false
multi_agent = false
browser_use = false
browser_use_external = false
browser_use_full_cdp_access = false
computer_use = false
in_app_browser = false
image_generation = false
view_image = false
memories = false
shell_snapshot = false
skill_search = false
skill_mcp_dependency_install = false
code_mode = false
code_mode_host = false
[analytics]
enabled = false
[feedback]
enabled = false
[mcp_servers]
[model_providers.fixture]
name = "local synthetic fixture"
base_url = "http://127.0.0.1:{port}/v1"
wire_api = "responses"
requires_openai_auth = false
[permissions.buzz.filesystem]
":root" = "deny"
":minimal" = "read"
":tmpdir" = "deny"
":slash_tmp" = "deny"
"{PROJECT}" = "read"
[permissions.buzz.filesystem.":workspace_roots"]
"." = "read"
[permissions.buzz.network]
enabled = false
'''
(HOME / 'config.toml').write_text(config)
env = {'HOME': str(HOME), 'CODEX_HOME': str(HOME), 'PATH': '/usr/bin:/bin', 'TMPDIR': str(ROOT), 'RUST_LOG': 'warn', 'HTTP_PROXY': 'http://127.0.0.1:9', 'HTTPS_PROXY': 'http://127.0.0.1:9', 'ALL_PROXY': 'http://127.0.0.1:9', 'NO_PROXY': '127.0.0.1,localhost', 'http_proxy': 'http://127.0.0.1:9', 'https_proxy': 'http://127.0.0.1:9', 'no_proxy': '127.0.0.1,localhost'}
child = subprocess.Popen([BINARY, 'app-server', '--strict-config'], cwd=PROJECT, env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
inbox = queue.Queue()
diagnostics = []


def pump(stream, stderr=False):
    total = 0
    while True:
        line = stream.readline(1000001)
        if not line:
            if not stderr:
                inbox.put({'fatal': 'child stdout closed'})
            return
        total += len(line)
        if total > 4000000 or len(line) > 1000000:
            inbox.put({'fatal': 'output budget exceeded'})
            return
        if stderr:
            diagnostics.append(line.decode(errors='replace'))
        else:
            try:
                inbox.put(json.loads(line))
            except ValueError:
                inbox.put({'fatal': 'invalid stdout JSON'})


threading.Thread(target=pump, args=(child.stdout,), daemon=True).start()
threading.Thread(target=pump, args=(child.stderr, True), daemon=True).start()
counter = 0


def send(value):
    child.stdin.write((json.dumps(value) + '\n').encode())
    child.stdin.flush()


def rpc(method, params):
    global counter
    counter += 1
    request_id = counter
    send({'id': request_id, 'method': method, 'params': params})
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        message = inbox.get(timeout=max(0.01, deadline - time.monotonic()))
        if 'fatal' in message:
            raise RuntimeError(message)
        if message.get('id') == request_id and ('result' in message or 'error' in message):
            return message
        if 'id' in message and 'method' in message:
            send({'id': message['id'], 'error': {'code': -32601, 'message': 'synthetic probe does not authorize side effects'}})
    raise TimeoutError(method)


try:
    print('EVIDENCE=' + str(ROOT), flush=True)
    print('INITIALIZE=' + json.dumps(rpc('initialize', {'clientInfo': {'name': 'buzz-scoped-probe', 'version': '0'}, 'capabilities': {'experimentalApi': True}})), flush=True)
    send({'method': 'initialized'})
    started = rpc('thread/start', {'cwd': str(PROJECT), 'model': 'fixture', 'modelProvider': 'fixture', 'permissions': 'buzz', 'approvalPolicy': 'on-request', 'approvalsReviewer': 'user', 'ephemeral': True, 'dynamicTools': [{'type': 'function', 'name': 'buzz_project_read', 'description': 'Synthetic project read', 'inputSchema': {'type': 'object', 'properties': {'path': {'type': 'string'}}, 'required': ['path'], 'additionalProperties': False}}]})
    if 'error' in started:
        raise RuntimeError(started)
    thread_id = started['result']['thread']['id']
    print('THREAD_PROFILE=' + json.dumps(started['result'].get('activePermissionProfile')), flush=True)
    print('THREAD_SANDBOX=' + json.dumps(started['result'].get('sandbox')), flush=True)
    print('TURN=' + json.dumps(rpc('turn/start', {'threadId': thread_id, 'permissions': 'buzz', 'approvalPolicy': 'on-request', 'approvalsReviewer': 'user', 'input': [{'type': 'text', 'text': 'Say hello.'}]})), flush=True)
    deadline = time.monotonic() + 20
    while time.monotonic() < deadline:
        message = inbox.get(timeout=max(0.01, deadline - time.monotonic()))
        if message.get('method') in ('error', 'turn/completed'):
            print('EVENT=' + json.dumps(message), flush=True)
            if message.get('method') == 'turn/completed':
                break
        if 'id' in message and 'method' in message:
            send({'id': message['id'], 'error': {'code': -32601, 'message': 'not authorized'}})
    print('MODEL_REQUESTS=' + str(len(CAPTURE)), flush=True)
    tool_outputs = []
    for request in CAPTURE:
        print('TOOLS=' + json.dumps([{'name': tool.get('name'), 'type': tool.get('type')} for tool in request.get('tools', [])]), flush=True)
        for item in request.get('input', []):
            if item.get('type') == 'function_call_output':
                print('ACTUAL_TOOL_OUTPUT=' + json.dumps(item), flush=True)
                tool_outputs.append(item['output'])
    (ROOT / 'model-requests.json').write_text(json.dumps(CAPTURE, indent=2))
    assert CAPTURE, 'No actual model request; not an execution proof'
    assert started['result']['activePermissionProfile']['id'] == 'buzz'
    if SHELL_PROBE:
        output = '\n'.join(tool_outputs)
        assert 'synthetic-inside' in output, 'Positive workspace read must succeed'
        assert 'synthetic-outside' not in output, 'Project policy leaked sibling data through direct or symlink read'
        assert not (PROJECT / 'model-forbidden.txt').exists(), 'Read-only project policy allowed a write'
    else:
        for request in CAPTURE:
            assert {tool.get('name') for tool in request['tools']} == {'request_user_input', 'buzz_project_read'}, 'Unexpected native tool exposed'
finally:
    (ROOT / 'stderr.txt').write_text(''.join(diagnostics))
    print('EXIT=' + str(child.poll()), flush=True)
    if child.poll() is None:
        child.kill()
        child.wait(timeout=5)
    server.shutdown()
    (ROOT / 'stderr.txt').write_text(''.join(diagnostics))
    print('STDERR=' + ''.join(diagnostics)[-5000:], flush=True)
