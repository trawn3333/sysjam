#!/usr/bin/env python3
"""Run sysjam and measure its own CPU usage (utime+stime of all threads)."""
import os, subprocess, sys, time, signal
secs = float(sys.argv[1]); args = sys.argv[2:]
clk = os.sysconf('SC_CLK_TCK')
def ticks(pid, tid=None):
    path = f'/proc/{pid}/stat' if tid is None else f'/proc/{pid}/task/{tid}/stat'
    f = open(path).read().rsplit(')', 1)[1].split()
    return int(f[11]) + int(f[12])
def per_thread(pid):
    # schedstat: exact on-CPU nanoseconds (tick-based utime/stime is coarse)
    out = {}
    for t in os.listdir(f'/proc/{pid}/task'):
        name = open(f'/proc/{pid}/task/{t}/comm').read().strip()
        ns = int(open(f'/proc/{pid}/task/{t}/schedstat').read().split()[0])
        out[t] = (name, ns)
    return out
p = subprocess.Popen(args, stdout=open('/tmp/measure.log', 'w'), stderr=subprocess.STDOUT)
time.sleep(3)                       # skip startup
t0, c0 = time.time(), ticks(p.pid); th0 = per_thread(p.pid)
time.sleep(secs)
t1, c1 = time.time(), ticks(p.pid); th1 = per_thread(p.pid)
rss = [l for l in open(f'/proc/{p.pid}/status') if l.startswith('VmRSS')][0].split()[1]
for t,(n,v) in th1.items():
    print(f"  thread {n:14s} {(v-th0.get(t,(n,0))[1])/1e9/(t1-t0)*100:.2f}% (schedstat)")
tot = sum(v for _,v in th1.values()) - sum(v for _,v in th0.values())
print(f"sysjam CPU (schedstat, exact): {tot/1e9/(t1-t0)*100:.2f}% of one core")
p.send_signal(signal.SIGINT); p.wait()
print(f"sysjam CPU: {(c1-c0)/clk/(t1-t0)*100:.2f}% of one core over {t1-t0:.0f}s, RSS {rss} kB")
