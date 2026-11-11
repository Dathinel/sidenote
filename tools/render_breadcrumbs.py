"""Inspect the shared breadcrumb in offscreen drill-down pages and parent navigation."""
from pathlib import Path
from concurrent.futures import ThreadPoolExecutor
import json
import render_settings_build as render
OUT=render.ROOT/'artifacts/settings-build/breadcrumbs';OUT.mkdir(parents=True,exist_ok=True);render.OUT=OUT
cases=[('meeting-long','screens-summary',None,'meeting-breadcrumb'),('meeting-parent','screens-summary',[render.tap('meeting-breadcrumb-parent')],None),('company','company-overview',None,'company-breadcrumb'),('notebook','notebook',None,'notebook-breadcrumb'),('tasks-home','tasks-v2-project',None,None),('memory-home','memory',None,None)]
project_events=[render.tap('nav-search',.2),dict(type='type',text='Northwind Atlas',t=.5),dict(type='key',name='Enter',t=1),render.tap('view-tasks',1.5)]
person_events=[render.tap('person-more-dana ortiz',.3),render.tap('person-menu-dana ortiz-item1',.7)]
cases += [('memory-all','memory',[render.tap('memory-browse')],'memory-breadcrumb'),('person','people-external',person_events,'person-breadcrumb'),('tasks-project','tasks-v2-project',project_events,'tasks-project-breadcrumb'),('memory-parent','memory',[render.tap('memory-browse'),render.tap('memory-breadcrumb-parent',.8)],None),('person-parent','people-external',person_events+[render.tap('person-breadcrumb-parent',1.2)],None),('tasks-parent','tasks-v2-project',project_events+[render.tap('tasks-project-breadcrumb-parent',2.)],None),('company-parent','company-overview',[render.tap('company-breadcrumb-parent')],None),('notebook-parent','notebook',[render.tap('notebook-breadcrumb-parent')],None)]
cases += [('annotations-list','annotations-list',None,None),('diary','diary',None,None)]
jobs=[(name,scene,width,theme,900,events) for name,scene,events,_ in cases for width in ((1280,390) if name in ('meeting-long','meeting-parent','company','notebook','tasks-home','memory-home') else (1280,)) for theme in ('dark','light')]
with ThreadPoolExecutor(max_workers=2) as pool:paths=list(pool.map(render.render,jobs))
for job in jobs:
 name,scene,width,theme,height,events=job
 if width!=390:continue
 elements=json.loads((OUT/f'settings-{name}-{width}-{theme}.json').read_text(encoding='utf-8-sig'))['elements']
 if any(n.get('key')=='sidebar-toggle' and n.get('label','').startswith('Hide') for n in elements):
  shifted=[dict(e,t=e['t']+.4) for e in (events or [])]
  render.render((name,scene,width,theme,height,[render.tap('sidebar-toggle',.2)]+shifted))
for name,_,_,expected in cases:
 for width in ((1280,390) if name in ('meeting-long','meeting-parent','company','notebook','tasks-home','memory-home') else (1280,)):
  for theme in ('dark','light'):
   elements=json.loads((OUT/f'settings-{name}-{width}-{theme}.json').read_text(encoding='utf-8-sig'))['elements']
   crumbs=[n for n in elements if n.get('label')=='Breadcrumb']
   if expected:
    assert len(crumbs)==1,(name,crumbs)
    assert crumbs[0].get('key')==expected,(name,crumbs)
    current=next(n for n in elements if n.get('key')==expected+'-current')
    parent=next(n for n in elements if n.get('key')==expected+'-parent')
    assert current.get('color')!=parent.get('color'),name
    assert current['rect'][0]+current['rect'][2]<=width+1,(name,current['rect'])
   else:assert not crumbs,(name,crumbs)
(OUT/'render-manifest.json').write_text(json.dumps(paths,indent=2)+'\n',encoding='utf-8')
print(f'PASS: all six quiet-chevron drill-downs, six parent actions, long-title bounds and top-level absence in {len(paths)} offscreen captures')
