"""Only invented data for offscreen tasks v2 renders."""
from pathlib import Path
import json, sys
root=Path(sys.argv[1])
folder=root/'tasks-v2-meetings'/'1800000000000-1'
folder.mkdir(parents=True,exist_ok=True)
def write(path,value):
    path.write_text(json.dumps(value,ensure_ascii=False),encoding='utf-8')
todos=[
    dict(id='draft',title='Review the field guide',description='Read the sample pages and check that the steps are clear. Bring the marked copy to the next planning meeting.',owner='Alex C',status='todo',team='Design',due='2026-11-10',subtasks=[dict(id='read',title='Read the sample pages',done=True),dict(id='mark',title='Mark the unclear steps',done=True),dict(id='share',title='Share the marked copy',done=False),dict(id='print',title='Print a clean copy',done=False)]),
    dict(id='send',title='Send the sample pack',description='Share the approved sample with the workshop team.',owner='Morgan Lee',status='waiting',waiting_on=dict(task_id='draft'),due='2026-11-11'),
    dict(id='copy',title='Check the workshop invitation',description='The draft is ready. Check the date and the reply address before it goes out.',owner='Alex Chen',status='review',due='2026-11-12'),
    dict(id='outside',title='Confirm the room booking',owner='Robin',status='waiting',waiting_on=dict(text="The venue team's reply"),due=None),
    dict(id='long',title='Prepare the revised workshop materials and check every printed sample before the next planning meeting',description='A deliberately long synthetic title checks wrapping at a narrow width.',owner=None,status='todo',due='2026-11-16'),
    dict(id='done',title='Book the print slot',owner='Morgan Lee',status='done',due='2026-11-09'),
    dict(task='Ask the supplier for a paper sample',owner='Robin',due=None),
]
for t in todos:
    if 'title' in t:t['task']=t['title']
write(folder/'english-summary.json',dict(title='Workshop planning',context=['Plan an invented paper workshop.'],todo=todos))
write(folder/'manifest.json',dict(status='complete',created_at_ms=1794195000000,duration_ms=1800000))
write(folder/'tasks.json',dict(meeting_project='Northwind Field Guide',tasks=[dict(i=i,project='Northwind Field Guide',mine=False) for i in range(len(todos))]))
write(root/'teams.json',dict(teams=[dict(name='Design',members=['Alex Chen','Jamie Park'],aliases={'Alex C':'Alex Chen'}),dict(name='Delivery',members=['Morgan Lee'],aliases={'M Lee':'Morgan Lee'})]))
