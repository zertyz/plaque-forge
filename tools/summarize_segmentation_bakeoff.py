#!/usr/bin/env python3
import argparse, json
from pathlib import Path

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('root', type=Path)
    ap.add_argument('--json', type=Path)
    ap.add_argument('--markdown', type=Path)
    ap.add_argument('--min-iou', type=float)
    ap.add_argument('--max-mean-absolute', type=float)
    a = ap.parse_args()

    rows = []
    for result in sorted(a.root.glob('*/result.json')):
        doc = json.loads(result.read_text())
        stages = doc.get('execution', [])
        rows.append({
            'backend': result.parent.name,
            'seconds': round(sum(float(s.get('seconds', 0)) for s in stages), 6),
            'devices': sorted({str(s.get('device')) for s in stages if s.get('device')}),
            'cache_hits': sum(1 for s in stages if s.get('cache_hit')),
            'process_peak_rss_mib': max((float(s.get('process_peak_rss_mib', 0)) for s in stages), default=0.0),
            'accelerator_peak_mib': max((float(s.get('accelerator_peak_mib', 0)) for s in stages), default=0.0),
            'version': doc.get('version'),
            'mean_confidence': doc.get('mean_confidence'),
            'minimum_confidence': doc.get('minimum_confidence'),
            'mean_coverage': doc.get('mean_coverage'),
            'maximum_coverage': doc.get('maximum_coverage'),
        })

    comparisons = []
    for path in sorted(a.root.glob('*-vs-*.json')):
        d = json.loads(path.read_text())
        comparisons.append({'name': path.stem, **d})

    report = {
        'format': 'plaque-forge.segmentation-bakeoff/1',
        'rows': rows,
        'comparisons': comparisons,
        'warning': 'Model similarity is not visual ground truth; promote only with render/homologation evidence.'
    }
    text = json.dumps(report, indent=2, sort_keys=True) + '\n'
    if a.json:
        a.json.write_text(text)

    md = [
        '# Segmentation bake-off',
        '',
        '| Backend | Seconds | Devices | Peak RSS MiB | Peak accelerator MiB | Cache hits |',
        '|---|---:|---|---:|---:|---:|'
    ]
    for r in rows:
        md.append(f"| {r['backend']} | {r['seconds']:.2f} | {','.join(r['devices'])} | {r['process_peak_rss_mib']:.1f} | {r['accelerator_peak_mib']:.1f} | {r['cache_hits']} |")

    if comparisons:
        md += [
            '',
            '## Mask Comparisons & Regression Analysis',
            '',
            '| Comparison | Frames | mIoU | Disagreement | Mean Abs Drift | P95 Abs Drift | Verdict |',
            '|---|---:|---:|---:|---:|---:|:---:|'
        ]
        for c in comparisons:
            iou = c.get('binary_at_0_5', {}).get('iou', 0.0)
            dis = c.get('binary_at_0_5', {}).get('disagreement_fraction', 0.0)
            mae = c.get('alpha', {}).get('mean_absolute', 0.0)
            p95 = c.get('alpha', {}).get('p95_absolute', 0.0)
            frames = c.get('frames', 0)
            verdict = "PASS"
            if a.min_iou is not None and iou < a.min_iou:
                verdict = "FAIL"
            if a.max_mean_absolute is not None and mae > a.max_mean_absolute:
                verdict = "FAIL"
            md.append(f"| {c['name']} | {frames} | {iou:.4f} | {dis:.4f} | {mae:.4f} | {p95:.4f} | **{verdict}** |")

    md += ['', '> Model similarity is not visual ground truth. Promotion requires render/homologation evidence.', '']
    if a.markdown:
        a.markdown.write_text('\n'.join(md))
    print(text, end='')

if __name__ == '__main__':
    main()
