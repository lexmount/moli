from collections import defaultdict
from pathlib import Path
import json
import statistics
import sys

root = Path(sys.argv[1])
result = json.loads((root / 'result.json').read_text())
by_stage = defaultdict(list)
for line in (root / 'owner-trace.log').read_text().splitlines():
    tag, stage, *values = line.split()
    assert tag == 'SPLIT3_TRACE' and len(values) == 6, line
    sequence, at, elapsed, depth, allocations, requested = map(int, values)
    by_stage[stage].append(dict(sequence=sequence, at_ns=at, elapsed_ns=elapsed,
                              observed_depth=depth, allocations=allocations,
                              requested_bytes=requested))


def stats(values):
    if not values:
        return {'count': 0}
    return {'count': len(values), 'median': statistics.median(values),
            'p95': statistics.quantiles(values, n=100, method='inclusive')[94]
            if len(values) > 1 else values[0], 'max': max(values)}


snapshots = by_stage['snapshot']
assert snapshots, 'the allocation workload must include diagnostic snapshots'
report = {'complete_probe': result['ok'], 'owner_queues': {},
          'allocation_scope': 'Successful Rust GlobalAlloc allocation/reallocation requests; excludes C++/V8 and does not count live bytes. Snapshot/trace observer work is included.',
          'timing_scope': 'Internal queue samples before the first retention snapshot include initialization, warmups, measured navigation and concurrent history reads. Record instrument overhead separately.',
          'allocation_samples': []}
for stage in ['owner-queue', 'owner-local-queue']:
    rows = [row for row in by_stage[stage] if row['at_ns'] < snapshots[0]['at_ns']]
    report['owner_queues'][stage] = {
        'queue_wait_us': stats([row['elapsed_ns'] / 1000 for row in rows]),
        'observed_waiting_depth': stats([row['observed_depth'] - 1 for row in rows]),
        'depth_scope': 'Queue length immediately after dequeue; excludes executing operation. This is the observed maximum, not an unsampled high-water guarantee.'}
commits = {row['sequence']: row for row in by_stage['document-commit']}
assert len(commits) == len(by_stage['document-commit']), 'commit sequence must be unique'
projections = defaultdict(list)
for row in by_stage['document-projection']:
    projections[row['sequence']].append(row)
matched = [sequence for sequence in commits if sequence in projections]
lags = [min(row['at_ns'] for row in projections[sequence]) - commits[sequence]['at_ns']
        for sequence in matched]
assert all(lag >= 0 for lag in lags), 'projection cannot precede physical commit'
report['document_commit_us'] = stats([row['elapsed_ns'] / 1000 for row in commits.values()])
report['commit_to_first_projection_us'] = stats([lag / 1000 for lag in lags])
report['unmatched_commits'] = sorted(set(commits) - set(projections))
report['unmatched_projections'] = sorted(set(projections) - set(commits))
report['multiple_projection_sequences'] = sorted(sequence for sequence, rows in projections.items() if len(rows) > 1)
report['transport_rejections'] = [
    {'incoming_bytes': row['sequence'], 'pending_bytes': row['elapsed_ns'],
     'pending_messages': row['observed_depth'], 'at_ns': row['at_ns']}
    for row in by_stage['output-admission-rejected']]
rejections = report['transport_rejections']
report['transport_rejections'] = {'count': len(rejections),
    'first': rejections[0] if rejections else None,
    'last': rejections[-1] if rejections else None}
report['snapshot_record_count'] = len(snapshots)
report['completed_snapshot_count'] = len(result['retention'])
for index, (sample, snapshot) in enumerate(zip(snapshots, result['retention'])):
    previous = snapshots[index - 1] if index else sample
    report['allocation_samples'].append({'label': snapshot['label'],
        'allocations': sample['allocations'], 'requested_bytes': sample['requested_bytes'],
        'allocation_delta': sample['allocations'] - previous['allocations'],
        'requested_byte_delta': sample['requested_bytes'] - previous['requested_bytes']})
(root / 'trace-summary.json').write_text(json.dumps(report, indent=2) + '\n')
print(json.dumps({key: value for key, value in report.items() if key != 'allocation_samples'}, indent=2))
