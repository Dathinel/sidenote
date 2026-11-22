"""Run every pipeline unit test with the pinned, hash-checked baseline available."""
import ast
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

root = Path(__file__).resolve().parents[1]
source = root / 'scripts/meeting-pipeline'
constants = {}
for node in ast.parse((source / 'test_routing.py').read_text()).body:
    if isinstance(node, ast.Assign) and isinstance(node.targets[0], ast.Name):
        if node.targets[0].id in ('BASELINE_REVISION', 'BASELINE_SHA256'):
            constants[node.targets[0].id] = ast.literal_eval(node.value)
subprocess.run(['git', '-C', str(root), 'merge-base', '--is-ancestor',
                constants['BASELINE_REVISION'], 'HEAD'], check=True)
ref = constants['BASELINE_REVISION'] + ':scripts/meeting-pipeline/meeting-audio.py'
baseline = subprocess.run(['git', '-C', str(root), 'show', ref], check=True,
                          capture_output=True).stdout
# The test independently verifies the exact baseline hash before importing it.
with tempfile.TemporaryDirectory(prefix='hex-pipeline-tests-') as folder:
    copied = Path(folder) / 'meeting-pipeline'
    shutil.copytree(source, copied, ignore=shutil.ignore_patterns('__pycache__'))
    (copied / 'baseline-audio.py').write_bytes(baseline)
    flags = getattr(subprocess, 'BELOW_NORMAL_PRIORITY_CLASS', 0)
    result = subprocess.run([sys.executable, '-m', 'unittest', 'discover', '-s', str(copied),
                             '-p', 'test_*.py', *sys.argv[1:]], creationflags=flags)
    sys.exit(result.returncode)
