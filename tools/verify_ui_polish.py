"""Verify the rendered native controls and filter persistence, using synthetic state only."""
import json
import sys
from pathlib import Path
from PIL import Image, ImageChops

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / 'artifacts/ui-polish/current'

def tree(name):
    value = json.loads((OUT / (name + '.json')).read_text(encoding='utf-8-sig'))
    assert not value['warnings'], (name, value['warnings'])
    return value['elements']

def keyed(nodes, key):
    return next(node for node in nodes if node.get('key') == key)

def main():
    index = json.loads((OUT / 'render-index.json').read_text())
    assert len(index) == 36, len(index)
    for shot in index:
        image = Image.open(shot['path'])
        assert image.size == (shot['width'], 800), (shot['path'], image.size)
        nodes = tree(Path(shot['tree']).stem)
        if shot['scene'] == 'button-sheet':
            for variant in ['Primary', 'Secondary', 'Ghost', 'Remove']:
                for state in ['Rest', 'Hover', 'Pressed', 'Focus', 'Disabled']:
                    node = keyed(nodes, f'sheet-{variant}-{state}')
                    assert node['rect'][3] == 32, (variant, state, node['rect'])
                    assert node['rect'][0] >= 0 and node['rect'][0] + node['rect'][2] <= shot['width'] + 1, node
                assert keyed(nodes, 'compact-' + variant)['rect'][3] == 28
            assert keyed(nodes, 'sheet-Primary-Rest')['fill'] == ('#ecececff' if shot['theme'] == 'dark' else '#1f2328ff')
        if shot['scene'] == 'memory':
            search = keyed(nodes, 'mem-search')['rect']
            assert search[2] <= 480.1, search
            for name in ['type', 'state', 'project']:
                keyed(nodes, 'memory-filter-' + name)
            assert keyed(nodes, 'memory-home')['role'] == 'button'
            assert not any(node.get('text') == 'Back to Memory' for node in nodes)
            if 'active' in Path(shot['path']).name:
                keyed(nodes, 'memory-clear-type')
                keyed(nodes, 'memory-clear-project')
                keyed(nodes, 'memory-clear-all')
                assert any(node.get('text') == 'Not confirmed' for node in nodes)
    differences = []
    for shot in index:
        name = Path(shot['path']).name
        before = OUT.parent / 'dev15' / name
        if before.exists():
            old, new = Image.open(before).convert('RGB'), Image.open(shot['path']).convert('RGB')
            difference = ImageChops.difference(old, new)
            changed = sum(pixel != (0, 0, 0) for pixel in difference.get_flattened_data())
            differences.append(dict(name=name, changed_pixels=changed, total_pixels=old.width * old.height))
    (OUT.parent / 'pixel-comparison.json').write_text(json.dumps(differences, indent=2))
    print(f'PASS: {len(index)} native render trees, control geometry, search cap, active filters, labelled metadata and {len(differences)} baseline comparisons')

if __name__ == '__main__':
    main()
