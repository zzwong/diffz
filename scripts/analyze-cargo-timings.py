#!/usr/bin/env python3
"""Summarize Cargo's embedded timing data without adding overlapping crate times."""
import argparse
from collections import defaultdict
import json
import math
from pathlib import Path
import re


def load_units(text):
    match = re.search(r'\bconst\s+UNIT_DATA\s*=\s*', text)
    if not match:
        raise ValueError('Cargo UNIT_DATA was not found')
    units, _ = json.JSONDecoder().raw_decode(text[match.end():])
    if not isinstance(units, list) or not units:
        raise ValueError('Cargo timing data has no completed units')
    ids = set()
    for unit in units:
        if not isinstance(unit, dict):
            raise ValueError('Timing units must be objects')
        for key in ('i', 'name', 'start', 'duration', 'sections', 'unblocked_units', 'unblocked_rmeta_units'):
            if key not in unit:
                raise ValueError(f'Missing timing field: {key}')
        if not isinstance(unit['i'], int) or isinstance(unit['i'], bool):
            raise ValueError('Timing unit IDs must be integers')
        if unit['i'] in ids:
            raise ValueError('Duplicate timing unit ID')
        ids.add(unit['i'])
        for field in ('start', 'duration'):
            value = unit[field]
            if not isinstance(value, (int, float)) or isinstance(value, bool) or not math.isfinite(value) or value < 0:
                raise ValueError('Timing values must be finite and nonnegative')
        unit_sections = unit['sections']
        if unit_sections is not None and not isinstance(unit_sections, list):
            raise ValueError('Timing sections must be a list or null')
        for entry in unit_sections or []:
            if not isinstance(entry, list) or len(entry) != 2 or not isinstance(entry[0], str) or not isinstance(entry[1], dict):
                raise ValueError('Malformed timing section')
            section = entry[1]
            if any(not isinstance(section.get(key), (int, float)) or isinstance(section[key], bool)
                   or not math.isfinite(section[key]) for key in ('start', 'end')):
                raise ValueError('Section values must be finite numbers')
            if not 0 <= section['start'] <= section['end'] <= unit['duration'] + 0.03:
                raise ValueError('Timing section is outside its unit')
    for unit in units:
        for field in ('unblocked_units', 'unblocked_rmeta_units'):
            if not isinstance(unit[field], list) or any(not isinstance(child, int) or isinstance(child, bool) for child in unit[field]):
                raise ValueError('Unblock edges must be lists of integer IDs')
            if any(child not in ids for child in unit[field]):
                raise ValueError('Unblock edge references a missing timing unit')
    return units


def sections(unit):
    return {name: section['end'] - section['start'] for name, section in unit.get('sections') or []}


def load_concurrency(text):
    match = re.search(r'\bconst\s+CONCURRENCY_DATA\s*=\s*', text)
    if not match:
        raise ValueError('Cargo CONCURRENCY_DATA was not found')
    data, _ = json.JSONDecoder().raw_decode(text[match.end():])
    validate_concurrency(data)
    return data


def validate_concurrency(data):
    if not isinstance(data, list) or not data:
        raise ValueError('Cargo concurrency data has no samples')
    for row in data:
        if not isinstance(row, dict) or not all(key in row for key in ('t', 'active', 'waiting', 'inactive')):
            raise ValueError('Missing concurrency fields')
        timestamp = row['t']
        if not isinstance(timestamp, (int, float)) or isinstance(timestamp, bool) or not math.isfinite(timestamp) or timestamp < 0:
            raise ValueError('Concurrency timestamps must be finite and nonnegative')
        if any(not isinstance(row[key], int) or isinstance(row[key], bool) or row[key] < 0 for key in ('active', 'waiting', 'inactive')):
            raise ValueError('Concurrency counts must be nonnegative integers')
    if any(a['t'] > b['t'] for a, b in zip(data, data[1:])):
        raise ValueError('Concurrency samples are out of order')


def summarize(units, concurrency):
    validate_concurrency(concurrency)
    by_id = {unit['i']: unit for unit in units}
    starts, ends = defaultdict(list), defaultdict(list)
    for unit in units:
        starts[unit['start']].append(unit['i'])
        ends[unit['start'] + unit['duration']].append(unit['i'])
    begin, finish = min(starts), max(ends)
    if concurrency[0]['t'] > begin + 0.03 or concurrency[-1]['t'] < finish - 0.03:
        raise ValueError('Concurrency samples do not cover the completed unit spans')
    active, exclusive, covered = set(), defaultdict(float), 0.0
    previous = min(starts)
    for now in sorted(set(starts) | set(ends)):
        interval = now - previous
        if active:
            covered += interval
        if len(active) == 1:
            exclusive[next(iter(active))] += interval
        for identifier in starts[now]:
            active.add(identifier)
        for identifier in ends[now]:
            active.remove(identifier)
        previous = now

    predecessors = defaultdict(list)
    for unit in units:
        for child in unit['unblocked_units']:
            predecessors[child].append((unit['start'] + unit['duration'], unit['i'], 'complete'))
        for child in unit['unblocked_rmeta_units']:
            frontend = next((value for name, value in unit.get('sections') or [] if name == 'frontend'), None)
            if frontend is None:
                raise ValueError('Metadata unblock edge has no frontend timing section')
            predecessors[child].append((unit['start'] + frontend['end'], unit['i'], 'metadata'))
    for child, incoming in predecessors.items():
        if any(parent == child or release > by_id[child]['start'] + 0.03
               for release, parent, _ in incoming):
            raise ValueError('Invalid or late unblock edge')

    remaining = {identifier: len(predecessors[identifier]) for identifier in by_id}
    ready = [identifier for identifier, count in remaining.items() if count == 0]
    visited = 0
    while ready:
        identifier = ready.pop()
        visited += 1
        for child in by_id[identifier]['unblocked_units'] + by_id[identifier]['unblocked_rmeta_units']:
            remaining[child] -= 1
            if remaining[child] == 0:
                ready.append(child)
    if visited != len(units):
        raise ValueError('Unblock edges contain a cycle')

    final = max(units, key=lambda unit: unit['start'] + unit['duration'])
    current, endpoint, chain, seen = final['i'], 'complete', [], set()
    while True:
        if current in seen:
            raise ValueError('Unblock edges contain a cycle')
        seen.add(current)
        unit = by_id[current]
        chain.append({'id': current, 'name': unit['name'], 'target': unit.get('target', ''),
                      'endpoint': endpoint, 'start': unit['start'], 'sections': sections(unit),
                      'duration': unit['duration']})
        incoming = predecessors[current]
        if not incoming:
            break
        release, parent, kind = max(incoming)
        if release > unit['start'] + 0.03:
            raise ValueError('Unblock edge occurs after the dependent started')
        chain[-1]['recorded_predecessors'] = [
            {'id': identifier, 'name': by_id[identifier]['name'], 'release': at, 'endpoint': point}
            for at, identifier, point in sorted(incoming)]
        chain[-1]['selected_predecessor'] = {'id': parent, 'release': release, 'endpoint': kind}
        chain[-1]['ready_gap_seconds'] = max(0.0, unit['start'] - release)
        current, endpoint = parent, kind
    chain.reverse()

    waiting = {
        'ready_wait_seconds': sum(b['t'] - a['t'] for a, b in zip(concurrency, concurrency[1:]) if a['waiting'] > 0),
        'max_ready_waiting': max(row['waiting'] for row in concurrency),
        'max_active_invocations': max(row['active'] for row in concurrency),
        'definition': 'Cargo ready units waiting for a slot; active invocation count is not compiler CPU utilization.',
    }
    rows = [{**unit, 'section_seconds': sections(unit), 'exclusive_span_seconds': exclusive[unit['i']]}
            for unit in units]
    return {'schema_version': 1, 'unit_count': len(units),
            'observed_span_seconds': finish - begin, 'active_union_seconds': covered,
            'summed_overlapping_unit_seconds': sum(unit['duration'] for unit in units),
            'definition': 'Unit spans overlap and are not CPU time. The gating path is one latest-release path through recorded unblock edges, with observed ready gaps, not a counterfactual speedup estimate. Exclusive spans have exactly one unit active and are not CPU time.',
            'observed_gating_path': chain,
            'concurrency': waiting,
            'units_by_duration': sorted(rows, key=lambda unit: unit['duration'], reverse=True)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('timing_html', type=Path)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    text = args.timing_html.read_text(encoding='utf-8')
    result = summarize(load_units(text), load_concurrency(text))
    args.output.write_text(json.dumps(result, indent=2, allow_nan=False) + '\n', encoding='utf-8')


if __name__ == '__main__':
    main()
