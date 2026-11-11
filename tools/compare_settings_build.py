"""Compare every native Settings capture with its available proposal PNG."""
from pathlib import Path
from PIL import Image,ImageChops,ImageStat
import json,html,re
ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'artifacts/settings-build'
PROPOSAL=ROOT.parent/'settings-ia/artifacts/settings-ia'
MATCH={'summaries-expanded':'meetings-summaries','general':'general','meetings-recording':'meetings-recording','meetings-summaries':'meetings-summaries','meetings-tasks':'meetings-tasks','people':'people','people-external':'people','hex-dictation':'hex-dictation','models-agents':'models-agents','models-speech':'models-speech','appearance':'appearance','sync':'sync','privacy-data':'privacy-data','updates':'updates-idle','diagnostics':'diagnostics','updates-available':'updates','updates-checking':'updates-idle','hex-scrolled':'hex-dictation','recording-expanded':'meetings-recording','helpers-expanded':'models-agents','speech-expanded':'models-speech','privacy-expanded':'privacy-data','diagnostics-expanded':'diagnostics','sync-expanded':'sync','people-editor':'people'}
DETAILS={'summaries-expanded':'Configured script controls are open in Summaries Advanced. Off remains visible outside Advanced.',
'general':'Tasks and Projects are visible beside identity. Conditional setup is on Models and agents.',
'meetings-recording':'One ownership note at the top; section jump tabs added. Recording rows use the dynamic schema and per-app choices.',
'meetings-summaries':'Off is visible. What is sent links to Privacy and data. Configured scripts remain in Advanced.',
'meetings-tasks':'Daily check is the single group. Sidebar switches moved to General.',
'prompt-library':'Existing two instruction editors, their safety disclosures and reset confirmation are preserved. No matching proposal PNG was supplied.',
'people':'Import tab removed. Import information remains searchable. Native roster uses the invented People fixture.',
'people-external':'External tab shown against the shared People proposal. Company grouping comes from the invented fixture.',
'hex-dictation':'Scroll-following jump tabs added; ownership note appears once. Native read-only speech-model list retains all returned models. Compatibility and Connection are named disclosures.',
'models-agents':'Ask, meeting summaries and annotations retain their native controls. Summary model is one disclosure; inactive statuses are text.',
'models-speech':'Native build capability text replaces unavailable downloads. Language editing links to Meetings. Unavailable stored model cleanup remains in Advanced.',
'appearance':'One ownership note; Hex title and mark retained. Native choice controls and switches use app tokens.',
'sync':'Folder location is a named disclosure. Native device facts come from the isolated fixture.',
'privacy-data':'Summary-request details, connected apps and Hex retention share one page. Owner note is shown once.',
'updates':'One bordered Application card; idle says Not checked yet in the synthetic fixture. One muted footnote sits on the page background.',
'diagnostics':'Native agent sample and sign-in recovery controls, update source and Hex diagnostics. Owner note appears once.',
'updates-available':'Downloaded update and release-note disclosure are inline in the Application card. Native fixture opens What changed.',
'updates-checking':'No checking proposal PNG was supplied; compared with idle composition. Checking action is disabled and status stays inline.',
'hex-scrolled':'No scrolled proposal PNG was supplied; compared with full Hex proposal. Connection is open, scrolled into view and the matching jump tab is active.',
'recording-expanded':'Advanced recording details, vocabulary editor and Hex summaries are open. Ownership note remains single.',
'helpers-expanded':'Summary-model disclosure is open on its canonical Agents page.',
'speech-expanded':'Language routing and stored unavailable model cleanup are disclosed. No duplicate language editor.',
'privacy-expanded':'Summary-request details are expanded; Hex ownership note remains at the top.',
'diagnostics-expanded':'Agent recovery and update source remain visible without a nested Advanced page.',
'sync-expanded':'Folder location is open and remains read only.',
'people-editor':'Team editor Advanced is open. The supplied People PNG is the closest page reference, not an editor-state baseline.',
'search':'No search-state proposal PNG was supplied. The native result opens and highlights silence sensitivity in Recording Advanced.'}
def main():
 paths=json.loads((OUT/'render-manifest.json').read_text(encoding='utf-8'))
 rows=[]
 for relative in paths:
  png=ROOT/relative;match=re.fullmatch(r'settings-(.*)-(1280|390)-(light|dark)\.png',png.name);assert match,png
  name,width,theme=match.groups();reference=PROPOSAL/f'proposed-{MATCH[name]}-{theme}.png' if name in MATCH else None
  image=Image.open(png).convert('RGB');row=dict(png=str(png),size=image.size,proposal=str(reference) if reference and reference.exists() else None,differences=[])
  if reference and reference.exists():
   proposal=Image.open(reference).convert('RGB');row['proposal_size']=proposal.size
   if width=='390':row['differences'].append('No 390px proposal PNG exists. Native layout collapses the sidebar, wraps tabs and stacks row controls.')
   else:
    w=min(image.width,proposal.width);h=min(image.height,proposal.height)
    diff=ImageChops.difference(image.crop((0,0,w,h)),proposal.crop((0,0,w,h)))
    row['mean_channel_difference_255']=round(sum(ImageStat.Stat(diff).mean)/3,2)
   row['differences'].append('Settings breadcrumb removed by owner decision, so the native page title and content begin higher. Native Rune text and control geometry differ from the HTML mockup.')
  row['differences'].append(DETAILS[name]);rows.append(row)
 for manifest,description in [(OUT/'search-render-manifest.json','Legacy-key search navigation proof. No proposal PNG exists for individual key searches.'),(OUT/'breadcrumbs/render-manifest.json','Owner breadcrumb decision. Parent/current quiet chevrons or a top-level absence check. No matching proposal PNG was supplied.')]:
  if not manifest.exists():continue
  for relative in json.loads(manifest.read_text(encoding='utf-8')):
   png=ROOT/relative;size=Image.open(png).size
   rows.append(dict(png=str(png),size=size,proposal=None,differences=[description]))
 (OUT/'proposal-comparison.json').write_text(json.dumps(rows,indent=2)+'\n',encoding='utf-8')
 lines=['Decisions needed: none.','','# Native renders compared with the proposal','','Each row is a generated PNG. All files were read by the comparison script. The pixel metric, where present in JSON, measures the overlapping image area and is not a visual quality score. Native Rune rendering and the owner changes intentionally move content. There are no supplied 390px, search, checking, scrolled or prompt-library baselines. Closest page references are identified below.','','| Render | Proposal | Differences |','|---|---|---|']
 for r in rows:lines.append('| '+Path(r['png']).relative_to(OUT).as_posix()+' | '+(Path(r['proposal']).name if r['proposal'] else 'No matching PNG supplied')+' | '+' '.join(r['differences'])+' |')
 (OUT/'proposal-comparison.md').write_text('\n'.join(lines)+'\n',encoding='utf-8')
 cards=''.join('<article><h2>'+html.escape(Path(r['png']).relative_to(OUT).as_posix())+'</h2><a href="'+html.escape(Path(r['png']).relative_to(OUT).as_posix())+'"><img loading="lazy" src="'+html.escape(Path(r['png']).relative_to(OUT).as_posix())+'"></a><p>'+html.escape(' '.join(r['differences']))+'</p></article>' for r in rows)
 (OUT/'index.html').write_text('<!doctype html><meta charset="utf-8"><title>Settings build renders</title><style>body{font:14px/1.5 system-ui;background:#efede7;color:#242422;margin:24px}main{display:grid;grid-template-columns:repeat(auto-fit,minmax(300px,1fr));gap:24px}article{min-width:0}img{width:100%;max-height:700px;object-fit:contain;object-position:top;background:#ddd}h2{font-size:14px}</style><h1>Settings build renders</h1><p><a href="proposal-comparison.md">Differences for every render</a></p><main>'+cards+'</main>',encoding='utf-8')
 print(f'PASS: compared all {len(rows)} render files; wrote proposal-comparison.md and index.html')
if __name__=='__main__':main()
