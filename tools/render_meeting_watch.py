"""Offscreen Labs proofs and legacy Settings reachability, using synthetic fixtures only."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import sys

import render_settings_build as settings


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('exe', type=Path)
    args = parser.parse_args()
    settings.EXE = args.exe.resolve()
    settings.OUT = settings.ROOT / 'artifacts/meeting-watch'
    settings.STATE = settings.OUT / 'render-state'
    settings.OUT.mkdir(parents=True, exist_ok=True)
    jobs = [(name, scene, 1280, 'light', height, None)
            for name, scene, height in settings.PAGES]
    for name, scene, events in [
        ('hex-scrolled', 'settings-hex-expanded', [settings.tap('settings-jump-5')]),
        ('recording-expanded', 'settings-recording-expanded', None),
        ('summaries-expanded', 'settings-summaries-expanded', None),
        ('helpers-expanded', 'settings-helpers-expanded', None),
        ('speech-expanded', 'settings-speech-expanded', None),
        ('people-editor', 'people-team-editor', [settings.tap('team-advanced')]),
    ]:
        jobs.append((name, scene, 1280, 'light', 2200, events))
    search_events = json.loads((settings.ROOT / 'tools/ui-scripts/settings-search.json').read_text())['events'][:-1]
    jobs.append(('search', 'settings-search', 1280, 'light', 800, search_events))
    for width in [1280, 390]:
        jobs.append(('labs', 'settings-diagnostics', width, 'light', 800, None))
        jobs.append(('labs-enabled', 'settings-diagnostics', width, 'light', 800, [
            settings.tap('labs-stop-forgotten-toggle', .3),
            settings.tap('labs-measure-files-toggle', .6),
        ]))
    paths = [settings.render(job) for job in jobs]
    for width in [1280, 390]:
        for mode in ['labs', 'labs-enabled']:
            base = f'settings-{mode}-{width}-light'
            tree = json.loads((settings.OUT / (base + '.json')).read_text(encoding='utf-8-sig'))
            elements = tree['elements']
            for key in ['labs-stop-forgotten', 'labs-silence-duration', 'labs-measure-files']:
                element = next(n for n in elements if n.get('key') == key)
                x, y, w, h = element['rect']
                assert x >= 0 and x + w <= width + 1 and y >= 0 and y + h <= 800, (base, key, element['rect'])
            text = ' '.join(n.get('text', '') for n in elements)
            assert "If a recording hears no speech for 20 minutes, Sidenote stops it so it doesn't run for hours." in text
            config = json.loads((settings.STATE / base / 'config.json').read_text(encoding='utf-8-sig'))
            labs = config.get('labs', {})
            enabled = mode == 'labs-enabled'
            assert labs.get('stop_forgotten_recordings', False) == enabled
            assert labs.get('measure_recording_files', False) == enabled
            for n in elements:
                if n.get('role') in ['button', 'switch', 'combobox'] and not n.get('inert'):
                    x, y, w, h = n['rect']
                    assert x >= -1 and x + w <= width + 1, (base, n.get('key'), n['rect'])
    subprocess.run([sys.executable, str(settings.ROOT / 'tools/settings-reachability.py'), str(settings.OUT)], check=True)
    (settings.OUT / 'render-index.json').write_text(json.dumps(dict(
        binary_sha256=hashlib.sha256(settings.EXE.read_bytes()).hexdigest(), paths=paths,
    ), indent=2), encoding='utf-8')
    print('PASS: Labs defaults, toggles and layout at 1280 and 390 px', flush=True)


if __name__ == '__main__':
    main()
