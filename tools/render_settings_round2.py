"""Final Settings review captures and the unchanged 49-key navigation proof."""
from concurrent.futures import ThreadPoolExecutor
import argparse,json,sys
import render_settings_build as r
r.OUT=r.ROOT/'artifacts/settings-build/round2'
r.STATE=r.ROOT/'target/settings-build-round2-render'

def main():
 parser=argparse.ArgumentParser()
 parser.add_argument('--search-keys',action='store_true')
 args=parser.parse_args()
 if args.search_keys:
  sys.argv=[sys.argv[0],'--search-keys']
  r.main()
  return
 r.OUT.mkdir(parents=True,exist_ok=True)
 jobs=[]
 for theme in ('dark','light'):
  for width in (1280,390):
   jobs += [('meetings-recording','settings-recording',width,theme,2200,None),('meetings-summaries','settings-summaries',width,theme,1800,[r.tap('hex-summaries')]),('hex-dictation','settings-hex',width,theme,2200,None),('hex-scrolled','settings-hex-expanded',width,theme,900,[r.tap('settings-jump-5')])]
 events=json.loads((r.ROOT/'tools/ui-scripts/settings-search.json').read_text())['events'][:-1]
 jobs += [('summaries-expanded','settings-summaries-expanded',1280,'light',2200,None),('search','settings-search',1280,'light',800,events),('updates','settings-updates',1280,'dark',900,None)]
 with ThreadPoolExecutor(max_workers=2) as pool: paths=list(pool.map(r.render,jobs))
 (r.OUT/'render-manifest.json').write_text(json.dumps(paths,indent=2)+'\n',encoding='utf-8')
 print(f'PASS: {len(paths)} isolated renders',flush=True)
if __name__=='__main__':main()
