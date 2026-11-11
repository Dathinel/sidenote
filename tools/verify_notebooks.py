"""Check notebook app renders and the actual static HTML proof artifacts."""
from pathlib import Path
import json,sys
from PIL import Image
root=Path(sys.argv[1]);count=0
for scene in ['notebooks','notebook','notebook-note','share-confirmation']:
 for width in [1280,640]:
  for theme in ['light','dark']:
   stem=f'{scene}-{width}-{theme}';tree=json.loads((root/f'{stem}.json').read_text(encoding='utf-8-sig'));assert not tree['warnings'],(stem,tree['warnings']);assert Image.open(root/f'{stem}.png').size==(width,800)
   nodes=tree['elements'];keys={n['key']:n for n in nodes if n.get('key')};texts=[n['text'] for n in nodes if n.get('text')]
   assert 'new-annotation' in keys and 'new-meeting' not in keys
   assert not any('helper' in t.lower() for t in texts)
   if scene=='notebooks':
    assert 'new-notebook' in keys and 'notebooks-list' in keys
    rows=[n for k,n in keys.items() if k.startswith('notebook-row-')];assert len(rows)==1
    assert rows[0]['rect'][1]<keys['annotation-row-0']['rect'][1]
   elif scene=='notebook':
    cards=sorted([n for k,n in keys.items() if k.startswith('notebook-capture-')],key=lambda n:n['rect'][1]);assert len(cards)==3
    assert all(abs(n['rect'][3]-120)<1 for n in cards)
    assert 'write-notebook-note' in keys and 'notebook-add-captures' in keys
    assert any('Alt+Up' in t and 'Alt+Down' in t for t in texts)
    assert any('no images' in t for t in texts)
   elif scene=='notebook-note':
    assert 'notebook-note-title' in keys and 'notebook-note-summary' in keys and 'share-notebook' in keys
    assert all(f'notebook-section-{i}' in keys for i in range(3))
    assert all(f'notebook-image-{i}-{j}' in keys for i in range(3) for j in (1,2))
    assert all(f'notebook-callout-{i}-{j}' in keys for i in range(3) for j in (1,2,3))
    assert 'Open questions' in texts and 'To-dos' in texts
    rect=keys['notebook-image-0-1']['rect'];px=int(rect[0]+(.31+.45/2)*rect[2]);py=int(rect[1]+(.06+.08/2)*rect[3])
    if 0<=px<width and 0<=py<800: assert Image.open(root/f'{stem}.png').convert('RGB').getpixel((px,py))==(0,0,0)
   else:
    assert 'HTML page saved' in texts and all(k in keys for k in ['share-copy-file','share-show-folder','share-back'])
   count+=1
proofs=[json.loads(p.read_text(encoding='utf-8')) for p in root.glob('export-proof-share-confirmation-*.json')];assert len(proofs)==4
assert all(p['only_approved_marked_png_bytes'] and p['image_count']==6 and p['scripts']==0 and p['absolute_paths']==0 and p['external_urls']==0 for p in proofs)
assert len({p['html_sha256'] for p in proofs})==1
edge=json.loads((root/'edge-renders.json').read_text(encoding='utf-8'));assert len(edge)==4 and all(p['headless'] and 'msedge.exe' in p['browser'].lower() for p in edge)
print(f'PASS: {count} app renders, zero warnings; all captures, frames, callouts and controls; four identical static exports with only approved PNG bytes; four Edge headless renders.')
