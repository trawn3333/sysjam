import numpy as np, sys
sr=44100
x=np.fromfile(sys.argv[1],dtype=np.float32).reshape(-1,2).mean(1)
segs=[(a,b) for a,b in [tuple(map(float,s.split('-'))) for s in sys.argv[2:]]] or [(0,len(x)/sr)]
bands=[("sub 20-60",20,60),("bass 60-250",60,250),("low-mid 250-800",250,800),("mid 800-3k",800,3000),("hi-mid 3-6k",3000,6000),("high 6-16k",6000,16000)]
for a,b in segs:
    y=x[int(a*sr):int(b*sr)]
    n=1<<15; w=np.hanning(n); acc=None; c=0
    for i in range(0,len(y)-n,n//2):
        p=np.abs(np.fft.rfft(y[i:i+n]*w))**2; acc=p if acc is None else acc+p; c+=1
    f=np.fft.rfftfreq(n,1/sr); acc/=c
    e={k:10*np.log10(acc[(f>=lo)&(f<hi)].sum()+1e-20) for k,lo,hi in bands}
    ref=e["mid 800-3k"]
    rms=20*np.log10(np.sqrt((y**2).mean())+1e-12); pk=20*np.log10(np.abs(y).max()+1e-12)
    print(f"{a:.0f}-{b:.0f}s  rms {rms:.1f} dBFS peak {pk:.1f} dBFS")
    for k,_,_ in bands: print(f"   {k:16s} {e[k]-ref:+6.1f} dB rel mid")
