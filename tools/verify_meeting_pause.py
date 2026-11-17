"""Render the pause meeting states with the app's invented offscreen scenes."""
import argparse
import json
import tempfile
from pathlib import Path
import render_people_review as native
import render_meeting_review as fixture


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('exe', type=Path)
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args()
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    native.OUT = out
    results = []
    assertions = 0
    for width in (1280, 640, 390):
        for name in ('recording', 'paused', 'transcript', 'resumed', 'writing', 'done', 'stale'):
            root = Path(tempfile.mkdtemp(prefix=name+'-', dir=out))
            meetings = fixture.prepare(root, 'finished')
            row = native.render(args.exe.resolve(), name, 'meeting-parts-'+name,
                                width, 'dark', state=root, meetings=meetings)
            tree = json.loads(Path(row['tree']).read_text(encoding='utf-8-sig'))
            assert not tree['warnings'], (name, width, tree['warnings'])
            nodes = {n['key']: n for n in tree['elements'] if n.get('key')}
            assert 'meeting-recording-control' in nodes, (name, width, 'missing shared control')
            assertions += 2
            if name in ('recording', 'transcript', 'resumed'):
                assert 'meeting-pause' in nodes and 'meeting-resume' not in nodes
                assert nodes['meeting-pause-square']['rect'][2:]==[12.0,12.0]
                assertions += 1
            elif name != 'writing':
                assert 'meeting-resume' in nodes
                assertions += 1
            if name == 'paused':
                assert any(n.get('text') == 'Generate notes' for n in tree['elements'])
                assert 'Paused' in nodes['meeting-recorded-time']['text']
                assertions += 2
            if name == 'stale':
                texts = [n.get('text','') for n in tree['elements']]
                assert 'New audio since these notes' in texts
                assert 'Update notes' in texts
                assertions += 2
            if name == 'resumed':
                assert any(n.get('text') == 'Recording resumed' for n in tree['elements'])
                assertions += 1
            if name == 'writing':
                assert 'meeting-writing-notes' in nodes
                assertions += 1
            for key in ('meeting-recording-control', 'meeting-transcript', 'meeting-pause', 'meeting-resume'):
                if key in nodes:
                    x,y,w,h = nodes[key]['rect']
                    assert x >= -1 and x+w <= width+1 and y >= 0 and y+h <= 801, (name,width,key,nodes[key]['rect'])
                    assertions += 1
            results.append(row)
    for width in (1280, 640, 390):
        root=Path(tempfile.mkdtemp(prefix='tooltip-',dir=out))
        meetings=fixture.prepare(root,'finished')
        row=native.render(args.exe.resolve(),'transcript-tooltip','meeting-reading-review',width,'dark',
                          [native.tap('meeting-transcript'),dict(type='move',key='meeting-transcript',t=.8)],root,meetings)
        tree=json.loads(Path(row['tree']).read_text(encoding='utf-8-sig'))
        assert not tree['warnings'],tree['warnings']
        nodes={n['key']:n for n in tree['elements'] if n.get('key')}
        tip=nodes['meeting-recording-tip-tooltip']['rect']
        copy=nodes['transcript-copy']['rect']
        assert tip[1]>=copy[1]+copy[3],(width,tip,copy)
        assert tip[1]+tip[3]<=800,(width,tip)
        results.append(row)
        assertions+=3
    (out / 'index.json').write_text(json.dumps(dict(scenes=results, assertions=assertions), indent=2))
    print(f'PASS: {len(results)} scenes, {assertions} assertions')

if __name__ == '__main__':
    main()
