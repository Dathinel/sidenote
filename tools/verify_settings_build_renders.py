"""Check rendered Settings structure, ownership, readable values and jump selection."""
from pathlib import Path
import json,sys
ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'artifacts/settings-build'
def load(name):
 value=json.loads((OUT/(name+'.json')).read_text(encoding='utf-8-sig'));assert not value['warnings'],(name,value['warnings']);return value['elements']
def keyed(elements,key):return next(n for n in elements if n.get('key')==key)
titles=set()
for path in OUT.glob('settings-*.json'):
 data=json.loads(path.read_text(encoding='utf-8-sig'))
 if 'elements' not in data:continue
 elements=data['elements'];assert not data['warnings'],(path.name,data['warnings'])
 texts=[n.get('text','') for n in elements]
 titles.update(n.get('text') for n in elements if n.get('key')=='settings-title')
 if path.name.startswith('settings-updates-'):assert not any('Hex is not running' in t for t in texts),path.name
 assert not any(t.startswith('Settings /') for t in texts),path.name
 assert texts.count('These settings live in Hex and also change Hex.')<=1,path.name
 assert not any(n.get('text')=='This PC' and '/hex-pane/' in n.get('path','') for n in elements),path.name
 assert not any('top_right' in t or 'always_record' in t or 'never_offer' in t for t in texts),path.name
 width=data['viewport'][0] if isinstance(data['viewport'],list) else data['viewport']['width']
 if width==390:
  for n in elements:
   if n.get('role') in ('button','combobox','textbox') and n['rect'][2]>0 and not n.get('inert'):
    x,y,w,h=n['rect'];assert x>=-1 and x+w<=width+1,(path.name,n.get('key'),n['rect'])
assert {'General','Meetings','Prompt library','People','Hex and dictation','Models and agents','Appearance and startup','Sync','Privacy and data','Updates','Diagnostics'}<=titles,titles
for theme in ('light','dark'):
 for width in (1280,390):
  base=f'settings-hex-scrolled-{width}-{theme}'
  elements=load(base)
  assert keyed(elements,'settings-jump-5').get('itemSelected'),base
  diagnostics=load(f'settings-diagnostics-{width}-{theme}')
  fixture=json.loads((ROOT/'tools/ui-fixtures/hex.json').read_text(encoding='utf-8'))
  build=next(st['value'] for st in fixture['settings'] if st['key']=='about.build')
  assert any(n.get('text')==build for n in diagnostics),(width,theme,'Hex build text')
  elements=load(f'settings-meetings-recording-{width}-{theme}')
  for index,label in enumerate(('Zoom','Microsoft Teams','Google Meet')):
   selector=keyed(elements,f'hex-call-app-{label}-{index}-trigger')
   assert selector.get('focusable') and not selector.get('disabled',False) and selector.get('opacity',1)>0.5,(label,selector)
  idle=load(f'settings-updates-{width}-{theme}')
  assert 'Update status' not in [n.get('text') for n in idle]
  assert not any(n.get('key')=='settings-advanced' for n in idle)
  assert keyed(idle,'updates-section')['rect'][3]>0
  ready=load(f'settings-updates-available-{width}-{theme}')
  assert keyed(ready,'update-install') and keyed(ready,'update-later') and keyed(ready,'update-notes')
  checking=load(f'settings-updates-checking-{width}-{theme}')
  assert any(n.get('text')=='Checking…' for n in checking)
print('PASS: both themes and widths, eleven page navigation, ownership, readable Hex values, active jump tabs, per-app controls and Updates states')
