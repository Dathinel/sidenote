"""Verify a real UI export before its isolated approval files are cleaned up."""
from pathlib import Path
from html.parser import HTMLParser
from collections import Counter
import base64,hashlib,json,re,sys
state,source,report=map(Path,sys.argv[1:4])
class Page(HTMLParser):
 def __init__(self): super().__init__();self.images=[];self.scripts=0;self.references=[];self.text=[]
 def handle_starttag(self,tag,attrs):
  if tag.lower()=='script': self.scripts+=1
  for key,value in attrs:
   if key.lower().startswith('on'): raise AssertionError('An event handler was exported')
   if key in ('src','href','action','poster','srcset'):
    self.references.append(value)
    assert tag=='img' and key=='src' and value.startswith('data:image/png;base64,'),(tag,key,value[:80])
    self.images.append(base64.b64decode(value.split(',',1)[1],validate=True))
 def handle_data(self,text): self.text.append(text)
html=source.read_text(encoding='utf-8');page=Page();page.feed(html)
assert page.scripts==0 and '<script' not in html.lower()
assert not re.search(r'(?i)https?://|file://|[a-z]:[\\/]|\\\\',html)
assert not re.search(r'(?i)@import|url\s*\(',html)
assert not re.search(r'(?:^|\s)/[A-Za-z0-9]', '\n'.join(page.text))
assert 'Made with Sidenote' in html and '@media print' in html and 'prefers-color-scheme:dark' in html
books=[json.loads(p.read_text(encoding='utf-8')) for p in (state/'annotations/notebooks').glob('*.json')]
assert len(books)==1;doc=books[0]['note'];expected=Counter();approved={}
for section,asset in zip(doc['value']['sections'],doc['assets']):
 assert asset['key'] not in html
 a=asset['approved'];assert a['approval']['approved_at_ms']>0
 frames={f['index']:f for f in a['approval']['frames']}
 for figure in section['figures']:
  frame=frames[figure['keyframe_index']]
  name=frame['marked'];data=(Path(a['folder'])/name).read_bytes();sha=hashlib.sha256(data).hexdigest();expected[sha]=1
  approved[sha]={'bytes':data,'approval_blake3':a['approval']['file_hashes'][name]}
  assert name not in html and a['folder'] not in html
actual=Counter()
for data in page.images:
 assert data.startswith(b'\x89PNG\r\n\x1a\n');sha=hashlib.sha256(data).hexdigest();assert sha in approved and data==approved[sha]['bytes'];actual[sha]+=1
assert actual==expected and len(page.images)==len(expected) and all(n==1 for n in actual.values()),(actual,expected)
assert len(page.images)==2 and 'Same screen as Workshop setup, marks 1, 2.' in html and 'Same screen as Workshop setup, mark 3.' in html
assert html.count('class="number"')==3
result={'scripts':page.scripts,'external_urls':0,'absolute_paths':0,'image_count':len(page.images),'only_approved_marked_png_bytes':True,'images':[{'sha256':sha,'approval_blake3':approved[sha]['approval_blake3'],'occurrences':count} for sha,count in sorted(actual.items())],'html_sha256':hashlib.sha256(source.read_bytes()).hexdigest()}
report.write_text(json.dumps(result,indent=2),encoding='utf-8');print('PASS: exported HTML has no scripts, URLs, absolute paths or internals; every unique PNG matches approved bytes and hashes, shared screens are referenced once.')
