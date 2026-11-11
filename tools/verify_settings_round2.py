"""Verify the final six Settings corrections in every requested native render."""
from pathlib import Path
import json
from PIL import Image,ImageChops
ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'artifacts/settings-build/round2'
MIXED='Settings marked ⬡ Hex live in Hex and also change Hex.'
HEX='These settings live in Hex and also change Hex.'

def load(name):
 tree=json.loads((OUT/(name+'.json')).read_text(encoding='utf-8-sig'))
 assert not tree['warnings'],(name,tree['warnings'])
 return tree['elements']
def keyed(nodes,key):return next(n for n in nodes if n.get('key')==key)

for theme in ('dark','light'):
 for width in (1280,390):
  for page in ('meetings-recording','meetings-summaries','hex-dictation'):
   name=f'settings-{page}-{width}-{theme}'
   nodes=load(name)
   texts=[n.get('text','') for n in nodes]
   keys={n.get('key') for n in nodes}
   if page.startswith('meetings-'):
    assert 'settings-tabs' in keys and 'settings-jump-tabs' not in keys,name
    assert [n['label'] for n in nodes if n.get('role')=='tab']==['Recording','Summaries','Tasks'],name
    assert texts.count(MIXED)==1 and HEX not in texts,name
    if page=='meetings-recording':
     assert 'hex-summaries' not in keys and 'hs-meetings.summaries' not in keys,name
     assert 'Hex and Sidenote spell these right in transcripts and summaries.' in texts,name
     switch=keyed(nodes,'hs-sw-meetings.enabled')
     track=next(n for n in nodes if n['path']==switch['path']+'/control')
     thumb=next(n for n in nodes if n['path']==track['path']+'/thumb')
     assert track['fill']==('#ecececff' if theme=='dark' else '#1f2328ff'),(name,track)
     assert thumb['fill']==('#1f2328ff' if theme=='dark' else '#ffffffff'),(name,thumb)
    else:
     assert 'hex-summaries' in keys and 'hs-meetings.summaries' in keys,name
     assert 'Hex · Summaries' in texts,name
   else:
    assert texts.count(HEX)==1 and MIXED not in texts,name
    assert not any(t.startswith('Hex · ') for t in texts),name
    assert 'settings-jump-tabs' in keys,name
    for key in ('hex-model-use-whisper-small','hex-model-install-whisper-large-v3'):
     action=keyed(nodes,key)
     assert action['role']=='button' and not action.get('disabled') and action.get('focusable'),(name,key,action)
    selected=[n for n in nodes if n.get('text')=='Selected']
    assert len(selected)==1 and selected[0].get('role')!='button',name
   if width==390:
    for n in nodes:
     if n.get('role') in ('button','combobox','textbox') and n['rect'][2]>0 and not n.get('inert'):
      x,y,w,h=n['rect'];assert x>=-1 and x+w<=width+1,(name,n.get('key'),n['rect'])
  scrolled=load(f'settings-hex-scrolled-{width}-{theme}')
  assert keyed(scrolled,'settings-jump-5').get('itemSelected'),(width,theme,'jump selection')
before=Image.open(ROOT/'artifacts/settings-build/settings-updates-1280-dark.png').convert('RGBA')
after=Image.open(OUT/'settings-updates-1280-dark.png').convert('RGBA')
assert before.size==after.size and ImageChops.difference(before,after).convert('RGB').getbbox() is None,'Updates changed'
print('PASS: all 12 requested renders, one Meetings tab row, summaries relocation, ownership, vocabulary, model actions, ink switches, narrow bounds, and pixel-identical Updates')
