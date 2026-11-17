import argparse
import ctypes
import http.server
import json
import os
from pathlib import Path
import struct
import sys
import tempfile
import threading
import time
import psutil
from idle_cpu_job import CpuJob


def system_times():
    values = [ctypes.c_ulonglong() for _ in range(3)]
    assert ctypes.windll.kernel32.GetSystemTimes(*(ctypes.byref(v) for v in values))
    return values[0].value, values[1].value + values[2].value


def measurement_self_test():
    job = CpuJob()
    try:
        proc = job.start([getattr(sys, '_base_executable', sys.executable), '-c', 'import time; time.sleep(0.2)'])
        started = time.monotonic()
        proc.wait(timeout=10)
        cpu, count, active = job.sample()
        assert proc.returncode == 0 and count >= 1
        assert time.monotonic() - started >= 0.1 and cpu >= 0
        assert system_times()[1] > 0
    finally:
        job.close()
    print('measurement self-test PASS', flush=True)


def seed(root, create_inputs=True):
    data, meetings = root / 'data', root / 'hex/meetings'
    for path in [data / 'local/notes-jobs', data / 'local/source-notes', meetings, root / 'peers', root / 'appdata']:
        path.mkdir(parents=True, exist_ok=True)
    (data / 'config.json').write_text(json.dumps({'version': 1, 'features': {}, 'notes': {'engine': 'app', 'helper': None}}))
    ids = [f'1800000000000-{i:03}' for i in range(148)]
    transcript_sizes = [64 * 1024, 256 * 1024, 1024 * 1024, 4 * 1024 * 1024]
    audio_sizes = [20, 40, 80, 150]
    for i, name in enumerate(ids):
        folder = meetings / name
        if create_inputs:
            folder.mkdir()
            manifest = {'status': 'complete', 'created_at_ms': 1800000000000}
            if i < 16:
                manifest['duration_ms'] = 1000
            (folder / 'manifest.json').write_text(json.dumps(manifest))
            line = json.dumps({'start_ms': 0, 'text': 'Synthetic words only. ' * 20}).encode() + b'\n'
            (folder / 'transcript.ndjson').write_bytes(line * max(1, transcript_sizes[i % 4] // len(line)))
            size = audio_sizes[i % 4] * 1024 * 1024
            with (folder / 'system.wav').open('wb') as output:
                output.write(struct.pack('<4sI4s4sIHHIIHH4sI', b'RIFF', size - 8, b'WAVE', b'fmt ', 16, 1, 1, 16000, 32000, 2, 16, b'data', size - 44))
                output.seek(size - 1)
                output.write(b'\0')
        if i < 19:
            job = dict(state='done', fallback=False, tasks_failed=False, attempts=1, next_try_at=None,
                       helper=None, model='', instructions='default', updated_at=0, message='synthetic', request=None)
            (data / 'local/notes-jobs' / f'{name}.json').write_text(json.dumps(job))
        if i < 91:
            (data / 'local/source-notes' / f'{name}.json').write_text(json.dumps({'notes': 'Synthetic source notes only.'}))
    cursor = {'delivered': ids[:16], 'revisions': {name: 'synthetic-seed-revision' for name in ids[:16]}}
    (data / 'local/recording-source.json').write_text(json.dumps(cursor))
    assert len(list(meetings.iterdir())) == 148
    assert len(list((data / 'local/notes-jobs').glob('*.json'))) == 19
    assert len(list((data / 'local/source-notes').glob('*.json'))) == 91
    return sum(p.stat().st_size for p in meetings.rglob('*') if p.is_file())


def run(args):
    measurement_self_test()
    if args.self_test:
        return
    assert args.seconds >= 600
    proof = (Path(tempfile.gettempdir()) / 'sidenote-idle-pass').resolve()
    if args.fixture:
        root = Path(args.fixture).resolve(strict=True)
        assert root.parent == proof and root.name.startswith('trial-')
        data = root / 'data'
        saved = root / ('saved-data-' + Path(args.output).stem)
        assert data.resolve().parent == root and not saved.exists()
        inventory = [(p, p.stat().st_size, p.stat().st_mtime_ns) for p in (root / 'hex/meetings').rglob('*') if p.is_file()]
        data.rename(saved)
        fixture_bytes = seed(root, create_inputs=False)
        assert all(p.stat().st_size == size and p.stat().st_mtime_ns == modified for p, size, modified in inventory)
        print(json.dumps(dict(reused_fixture=str(root), saved_prior_data=str(saved), unchanged_input_files=len(inventory))), flush=True)
    else:
        root = Path(tempfile.mkdtemp(prefix='trial-', dir=proof))
        fixture_bytes = seed(root)
    stopping = threading.Event()
    calls = {'sse_connections': 0, 'sse_keepalives': 0, 'status': 0}
    class Handler(http.server.BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass
        def do_GET(self):
            assert self.path == '/meetings/events'
            calls['sse_connections'] += 1
            self.send_response(200)
            self.send_header('Content-Type', 'text/event-stream')
            self.end_headers()
            try:
                while not stopping.is_set():
                    self.wfile.write(b': synthetic keepalive\n\n')
                    self.wfile.flush()
                    calls['sse_keepalives'] += 1
                    stopping.wait(10)
            except (ConnectionError, OSError):
                pass
        def do_POST(self):
            self.rfile.read(int(self.headers.get('Content-Length', '0')))
            calls['status'] += 1
            body = json.dumps(dict(version=4, status='status', app_version='synthetic', meeting={'state': 'idle'}, capture={'state': 'idle'}, model={}, capabilities={})).encode()
            self.send_response(200)
            self.send_header('Content-Length', str(len(body)))
            self.end_headers()
            self.wfile.write(body)
    server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
    (root / 'hex/local-api.json').write_text(json.dumps({'port': server.server_port, 'token': 'synthetic-idle-pass'}))
    threading.Thread(target=server.serve_forever, daemon=True).start()
    paths = ['--data-dir', str(root / 'data'), '--meetings-dir', str(root / 'hex/meetings'), '--peers-dir', str(root / 'peers')]
    env = dict(os.environ, SIDENOTE_IDLE_PASS='1', SIDENOTE_DATA_DIR=str(root / 'data'), SIDENOTE_HEX_DIR=str(root / 'hex/meetings'), SIDENOTE_PEERS_DIR=str(root / 'peers'), APPDATA=str(root / 'appdata'))
    job = CpuJob()
    with Path(args.output + '.counters').open('w') as counters:
        before_system = system_times()
        started = time.monotonic()
        proc = job.start([str(Path(args.exe).resolve()), '--listen', *paths], env=env, stdout=counters, stderr=counters)
        process = psutil.Process(proc.pid)
        initial_io = process.io_counters()
        print(json.dumps(dict(started_pid=proc.pid, fixture=str(root), fixture_bytes=fixture_bytes, fake_sse_port=server.server_port)), flush=True)
        try:
            for minute in range((args.seconds + 59) // 60):
                stopping.wait(min(60, max(0, started + args.seconds - time.monotonic())))
                assert proc.poll() is None, 'recorder exited'
                cpu = process.cpu_times()
                elapsed = time.monotonic() - started
                print(json.dumps(dict(elapsed=elapsed, cpu_seconds=cpu.user + cpu.system, one_core_percent=100 * (cpu.user + cpu.system) / elapsed, **calls)), flush=True)
            cpu = process.cpu_times()
            total_cpu, total_processes, active = job.sample()
            elapsed = time.monotonic() - started
            after_system = system_times()
            io = process.io_counters()
            cursor = json.loads((root / 'data/local/recording-source.json').read_text())
            result = dict(seconds=elapsed, recorder_cpu_seconds=cpu.user + cpu.system, one_core_percent=100 * (cpu.user + cpu.system) / elapsed,
                          tree_cpu_seconds=total_cpu, total_processes=total_processes, system_load_percent=100 * (1 - (after_system[0] - before_system[0]) / (after_system[1] - before_system[1])),
                          read_bytes=io.read_bytes - initial_io.read_bytes, read_count=io.read_count - initial_io.read_count,
                          fixture=str(root), fixture_bytes=fixture_bytes, delivered=len(cursor['delivered']), revisions=len(cursor['revisions']),
                          notes_jobs=len(list((root / 'data/local/notes-jobs').glob('*.json'))), source_notes=len(list((root / 'data/local/source-notes').glob('*.json'))), **calls)
            assert calls['sse_connections'] == 1 and calls['sse_keepalives'] >= 50
            assert result['delivered'] == result['revisions'] == 16
            assert result['notes_jobs'] == 19 and result['source_notes'] == 91
            counters.flush()
            snapshots = [json.loads(line.removeprefix('idle-pass ')) for line in Path(args.output + '.counters').read_text().splitlines() if line.startswith('idle-pass ')]
            stages = snapshots[-1]['cumulative']
            assert stages['watch_pass']['calls'] >= 10 and stages['listener_pass']['calls'] >= 10
            assert stages['file_reads']['bytes'] > 0 and stages['cursor_reads']['calls'] >= 20
            assert stages['notes_queue']['calls'] >= 10 and stages['sse_handling']['calls'] >= 90
            result['stage_counters_at_last_completed_pass'] = stages
            Path(args.output).write_text(json.dumps(result, indent=2))
            print(json.dumps(result), flush=True)
        finally:
            job.close()
            proc.wait()
            stopping.set()
            server.shutdown()


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('exe', nargs='?')
    parser.add_argument('--seconds', type=int, default=600)
    parser.add_argument('--output', default='idle-pass-result.json')
    parser.add_argument('--self-test', action='store_true')
    parser.add_argument('--fixture')
    args = parser.parse_args()
    (Path(tempfile.gettempdir()) / 'sidenote-idle-pass').mkdir(exist_ok=True)
    run(args)
