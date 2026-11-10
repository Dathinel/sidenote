"""Invented meeting rosters, teams and rollout preferences for People checks."""
from pathlib import Path
from datetime import datetime
import json


def prepare(root):
    root = Path(root)
    meetings = root / 'people-meetings'
    rows = [
        ('1800000000000-1', 'Earlier planning', '2026-11-08T10:00:00+05:30', ['Alex C', 'Dana Ortiz', 'Old Only'], [('old', 'Review the old draft', 'Alex C', 'todo')]),
        ('1800000000001-1', 'Workshop planning', '2026-11-10T10:00:00+05:30', ['Alex C', 'Jamie Park', 'Morgan Lee', 'Dana Ortiz'], [('review', 'Review the field guide', 'Alex C', 'todo'), ('send', 'Send the sample pack', 'Morgan Lee', 'waiting'), ('reply', 'Confirm the venue reply', 'Dana Ortiz', 'waiting')]),
        ('1800000000002-1', 'Workshop check-in', '2026-11-11T10:00:00+05:30', ['Alex Chen', 'Jamie Park', 'M Lee', 'Dana Ortiz', 'Leo Park'], [('copy', 'Share the marked copy', 'Alex Chen', 'todo'), ('venue', 'Confirm the venue booking', 'Dana Ortiz', 'todo'), ('done', 'Book the print slot', 'Morgan Lee', 'done'), ('legacy-owner', 'An old owner is only mentioned', 'Old Only', 'todo')]),
    ]
    for ident, title, date, people, todos in rows:
        folder = meetings / ident
        folder.mkdir(parents=True, exist_ok=True)
        items = [dict(id=key, task=text, title=text, owner=owner, status=status) for key, text, owner, status in todos]
        (folder/'english-summary.json').write_text(json.dumps(dict(schema=2, title=title, people=people, todo=items)))
        (folder/'manifest.json').write_text(json.dumps(dict(status='complete', created_at_ms=int(datetime.fromisoformat(date).timestamp()*1000), duration_ms=1800000)))
        (folder/'tasks.json').write_text(json.dumps(dict(meeting_project='Northwind Field Guide', tasks=[dict(i=i, mine=False, project='Northwind Field Guide') for i in range(len(items))])))
    if not (root/'teams.json').exists():
        (root/'teams.json').write_text(json.dumps(dict(teams=[dict(name='Design', members=['Alex Chen', 'Jamie Park'], aliases={'Alex C':'Alex Chen'}), dict(name='Delivery', members=['Morgan Lee'], aliases={'M Lee':'Morgan Lee'})])))
    (root/'local').mkdir(exist_ok=True)
    path = root/'local/ui.json'
    if not path.exists():
        path.write_text(json.dumps(dict(people_existing_meetings=['local/1800000000000-1'], people_since_ms=int(datetime.fromisoformat('2026-11-09T00:00:00+05:30').timestamp()*1000), people_sides={'dana ortiz':'external', 'leo park':'external'})))
    return meetings
