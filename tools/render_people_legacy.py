"""Render all legacy Settings destinations without changing their reachability gate."""
import argparse
import json
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
import render_people_review as review


def main():
    p = argparse.ArgumentParser()
    p.add_argument('exe', type=Path)
    a = p.parse_args()
    review.OUT = review.OUT/'legacy'
    scenes = ['general', 'recording', 'summaries', 'tasks', 'prompts', 'people', 'hex', 'helpers', 'speech', 'helper-advanced', 'appearance', 'sync', 'updates', 'search', 'sidebar-open', 'sidebar-folded', 'recording-expanded', 'summaries-expanded', 'tasks-expanded', 'hex-expanded', 'helper-advanced-expanded', 'sync-expanded', 'updates-expanded', 'helpers-expanded']
    search = [e for e in json.loads((review.ROOT/'tools/ui-scripts/settings-search.json').read_text())['events'] if e['type'] != 'mark']
    jobs = [('settings-'+name, 'settings-'+name, search if name == 'search' else None) for name in scenes]
    jobs.append(('settings-people-editor', 'people-team-editor', [review.tap('team-advanced')]))
    with ThreadPoolExecutor(max_workers=2) as pool:
        list(pool.map(lambda job:review.render(a.exe.resolve(), job[0], job[1], events=job[2]), jobs))
    print('PASS: 25 legacy Settings destination renders', flush=True)


if __name__ == '__main__':
    main()
