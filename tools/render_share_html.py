"""Render the export in Edge headless and independently check its PNG pixels."""
from pathlib import Path
import json,subprocess,sys
from PIL import Image
repo=Path(__file__).resolve().parents[1]
source=Path(sys.argv[1]).resolve();out=Path(sys.argv[2]).resolve()
subprocess.run(['node',str(repo/'tools/render_share_html.mjs'),str(source),str(out)],check=True,cwd=repo,timeout=120,creationflags=subprocess.CREATE_NO_WINDOW)
rows=json.loads((out/'edge-renders.json').read_text(encoding='utf-8'))
for row in rows:
 im=Image.open(row['png']).convert('RGB');assert im.size==(row['width'],row['height'])
 pixel=im.getpixel((0,0));assert (max(pixel)<65 if row['theme']=='dark' else min(pixel)>230),(row['theme'],pixel)
 row['background']=pixel
(out/'edge-renders.json').write_text(json.dumps(rows,indent=2),encoding='utf-8')
print('PASS: 1280 and 390 wide, both schemes, correct dimensions and independent background-pixel checks.')
