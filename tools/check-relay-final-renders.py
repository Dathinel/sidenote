"""Check the final preview's terminal and uncertain pairing states."""
import json
from pathlib import Path
import struct
import sys

folder=Path(sys.argv[1])
timeout="Pairing timed out. Start again on both devices."
uncertain="The other device may have joined. Check it. If you don't trust it, turn off relay sync on all devices and pair again."
lost="A lost or stolen device can't be removed in this preview. If that happens, turn off relay sync on all devices."
for theme in ('light','dark'):
    for scene in ('timeout','unconfirmed','preview'):
        stem=folder/f'relay-{scene}-{theme}'
        assert struct.unpack('>II',stem.with_suffix('.png').read_bytes()[16:24])==(1280,800)
        frame=json.loads(stem.with_suffix('.json').read_text(encoding='utf-8-sig'))
        assert not frame['warnings'],stem
        elements=frame['elements']; text=[node['text'] for node in elements if 'text' in node]
        assert any(lost in value for value in text),stem
        if scene=='timeout':
            assert timeout in text,stem
            assert not any(node.get('key') in ('relay-confirm','relay-safety-phrase','relay-invitation-code') for node in elements),stem
            notice=next(node for node in elements if node.get('key')=='relay-timeout-message')
        elif scene=='unconfirmed':
            assert 'Pairing not confirmed' in text and uncertain in text,stem
            notice=next(node for node in elements if node.get('key')=='relay-unconfirmed-notice-1')
        else:
            continue
        assert 58<=notice['rect'][1] and notice['rect'][1]+notice['rect'][3]<=800,stem
print('PASS: six final preview PNGs; timeout clears the old prompt, uncertain pairing and lost-device copy are present.')
