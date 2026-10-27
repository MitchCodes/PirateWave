"""Compare repeated game captures in milliseconds; no game injection or capture.

Use the same scene, capture duration and Discord activity for every run.
The input CSV column and process filter are explicit to avoid comparing the
wrong application's frames. This is a development acceptance tool, not PirateWave.
"""
import argparse
import csv
import hashlib
import json
import math
import os
from pathlib import Path
import statistics


def percentile(values,p):
    position=(len(values)-1)*p
    low=math.floor(position);high=math.ceil(position)
    return values[low]+(values[high]-values[low])*(position-low)


def metrics(values):
    if len(values)<1000 or any(not math.isfinite(v) or v<=0 for v in values):
        raise ValueError('Need at least 1000 finite, positive frame times in milliseconds')
    values=sorted(values)
    tail=values[-math.ceil(len(values)*0.01):]
    return {'frames':len(values),'p95_ms':percentile(values,.95),
            'p99_ms':percentile(values,.99),
            'one_percent_low_fps':statistics.mean(1000/v for v in tail)}


def read(path,column,process):
    with path.open(encoding='utf-8-sig',newline='') as stream:
        rows=list(csv.DictReader(stream))
    if process:
        rows=[r for r in rows if Path(r.get('Application',r.get('ProcessName',''))).name.casefold()==process.casefold()]
    result=metrics([float(r[column]) for r in rows])
    return {'file':str(path),'sha256':hashlib.sha256(path.read_bytes()).hexdigest(),**result}


def compare(baseline,enabled):
    if len(baseline)<3 or len(enabled)<3:raise ValueError('Use at least three runs per condition')
    base_p95=[r['p95_ms'] for r in baseline];base_low=[r['one_percent_low_fps'] for r in baseline]
    on_p95=statistics.median(r['p95_ms'] for r in enabled)
    on_low=statistics.median(r['one_percent_low_fps'] for r in enabled)
    variation_p95=(max(base_p95)-min(base_p95))/statistics.median(base_p95)*100
    variation_low=(max(base_low)-min(base_low))/statistics.median(base_low)*100
    reliable=max(variation_p95,variation_low)<=5
    return {'baseline_p95_range_ms':[min(base_p95),max(base_p95)],
            'baseline_one_percent_low_range_fps':[min(base_low),max(base_low)],
            'baseline_variation_percent':{'p95':variation_p95,'one_percent_low':variation_low},
            'enabled_median_p95_ms':on_p95,'enabled_median_one_percent_low_fps':on_low,
            'p95_degradation_percent':(on_p95/statistics.median(base_p95)-1)*100,
            'one_percent_low_degradation_percent':(1-on_low/statistics.median(base_low))*100,
            'pass':reliable and on_p95<=max(base_p95)*1.05 and on_low>=min(base_low)*.95,
            'inconclusive':not reliable,
            'method':'Enabled medians must remain within 5% of the worst baseline run; baseline range >5% is inconclusive. 1% low is the average FPS of the slowest 1% of frames.'}

def validate_capture_paths(baseline,enabled):
    paths=[path.resolve() for path in [*baseline,*enabled]]
    if len(paths)!=len(set(paths)):
        raise ValueError('Each run needs a distinct capture; baseline and enabled files cannot overlap')


def main():
    p=argparse.ArgumentParser()
    p.add_argument('--baseline',type=Path,nargs='+',required=True)
    p.add_argument('--enabled',type=Path,nargs='+',required=True)
    p.add_argument('--column',required=True,help='Frame time column in milliseconds')
    p.add_argument('--process',help='Exact game executable filename')
    p.add_argument('--out',type=Path,required=True)
    args=p.parse_args()
    if args.out.exists():p.error('Use a new report filename')
    validate_capture_paths(args.baseline,args.enabled)
    baseline=[read(path,args.column,args.process) for path in args.baseline]
    enabled=[read(path,args.column,args.process) for path in args.enabled]
    report={'schema':1,'cpu_identifier':os.environ.get('PROCESSOR_IDENTIFIER',''),
            'baseline':baseline,'enabled':enabled,**compare(baseline,enabled)}
    args.out.write_text(json.dumps(report,indent=2),encoding='utf-8')
    print(json.dumps(report,indent=2))
    return 0 if report['pass'] else 1


if __name__=='__main__':raise SystemExit(main())
