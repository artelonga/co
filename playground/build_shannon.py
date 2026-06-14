#!/usr/bin/env python3
"""SHANNON v2 — Everything Is Bits.
An information-theoretic instrument over the REAL ArteLonga universe: 577 commits
across co/yggdrasil/quilomboaraucaria. Five lenses, one unit (the bit):
multi-repo bandwidth (bits/s), redundancy (compression), re-identification (privacy),
mutual information (correlated leakage), secret strength (unguessability).
Self-contained HTML. Nothing leaves the page.
"""
import json, math, re, zlib
from collections import Counter, defaultdict
from datetime import datetime, timezone

def H(counts):
    n = sum(counts.values())
    return -sum((c/n)*math.log2(c/n) for c in counts.values() if c) if n else 0.0

def bits_per_char(s):
    return H(Counter(s))

REPO_COLORS = {"CO":"#22d3ee","YG":"#f472b6","QB":"#4ade80"}
TYPE_RE = re.compile(r'^(feat|fix|docs|refactor|chore|test|perf|build|ci)')

rows = []
for line in open("/tmp/multi.tsv", encoding="utf-8", errors="replace"):
    line = line.rstrip("\n")
    p = line.split("\t")
    if len(p) < 3: continue
    try: ts = int(p[0])
    except ValueError: continue
    repo, subj = p[1], p[2]
    m = TYPE_RE.match(subj)
    ctype = m.group(1) if m else "other"
    rows.append({"t":ts,"repo":repo,"subj":subj,"type":ctype})
rows.sort(key=lambda r:r["t"])

# ---- Panel 1: multi-repo bandwidth -----------------------------------------
series=[]; total=0.0; peak={"rate":0,"subj":"","repo":""}
for i,r in enumerate(rows):
    bits = bits_per_char(r["subj"])*len(r["subj"])
    total += bits
    rate = 0.0 if i==0 else bits/max(1, r["t"]-rows[i-1]["t"])
    if rate>peak["rate"]: peak={"rate":rate,"subj":r["subj"][:64],"repo":r["repo"]}
    series.append({"r":round(rate,3),"repo":r["repo"],"subj":r["subj"][:72],"type":r["type"]})
span=max(1, rows[-1]["t"]-rows[0]["t"])

# today's output (UTC day of latest commit)
day0 = datetime.fromtimestamp(rows[-1]["t"], timezone.utc).replace(hour=0,minute=0,second=0).timestamp()
today = [r for r in rows if r["t"]>=day0]
today_bits = sum(bits_per_char(r["subj"])*len(r["subj"]) for r in today)
today_by = Counter(r["repo"] for r in today)

panel1={"series":series,"total":round(total),"n":len(rows),"span_h":round(span/3600,1),
        "avg":round(total/span,3),"peak":round(peak["rate"],1),"peak_subj":peak["subj"],
        "peak_repo":peak["repo"],"by_repo":dict(Counter(r["repo"] for r in rows)),
        "today_n":len(today),"today_bits":round(today_bits),"today_by":dict(today_by)}

# ---- Panel 3: redundancy / compression -------------------------------------
corpus = "\n".join(r["subj"] for r in rows).encode()
raw=len(corpus); gz=len(zlib.compress(corpus,9))
order0 = H(Counter(corpus))  # bits/byte if encoded by frequency alone
order0_bytes = round(order0/8*raw)
panel3={"raw":raw,"gzip":gz,"order0":round(order0,2),"order0_bytes":order0_bytes,
        "redundancy":round((1-gz/raw)*100,1)}

# ---- Panel 5: mutual information  (repo ; commit-type) ----------------------
joint=Counter((r["repo"],r["type"]) for r in rows)
repos=sorted({r["repo"] for r in rows}); types=sorted({r["type"] for r in rows})
N=len(rows)
px=Counter(r["repo"] for r in rows); py=Counter(r["type"] for r in rows)
mi=0.0
for (x,y),c in joint.items():
    pxy=c/N
    mi += pxy*math.log2(pxy/((px[x]/N)*(py[y]/N)))
Hy=H(py)
matrix=[[joint.get((x,y),0) for y in types] for x in repos]
panel5={"mi":round(mi,3),"Hy":round(Hy,3),"repos":repos,"types":types,"matrix":matrix,
        "frac":round(mi/Hy*100,1) if Hy else 0}

# ---- Panel 2: privacy fields & Panel 4: secret presets ---------------------
fields=[
  {"k":"country","b":7.6,"on":True},{"k":"city","b":17.0,"on":True},
  {"k":"browser+ver","b":7.0,"on":True},{"k":"os","b":2.7,"on":False},
  {"k":"referrer","b":5.0,"on":False},{"k":"event_type","b":2.0,"on":False},
  {"k":"universe","b":9.0,"on":False},{"k":"ip /24","b":24.0,"on":False},
  {"k":"al_vid (cookie)","b":53.0,"on":False},
]
presets=[{"label":"password123","v":"password123"},
  {"label":"xkcd passphrase","v":"correct horse battery staple"},
  {"label":"CO token (32 hex)","v":"a3f1c09b7e264d8a15bf90e6c2d47a8b"},
  {"label":"UUIDv4","v":"f47ac10b-58cc-4372-a567-0e02b2c3d479"}]

DATA={"p1":panel1,"p3":panel3,"p5":panel5,"fields":fields,
      "earth":round(math.log2(8.1e9),1),"presets":presets,"colors":REPO_COLORS}

TPL=open("/Users/artelonga/projects/co/playground/_shell.html",encoding="utf-8").read()
out=TPL.replace("__DATA__", json.dumps(DATA))
open("/Users/artelonga/projects/co/playground/shannon.html","w",encoding="utf-8").write(out)
open("/Users/artelonga/projects/co/co-web/static/shannon/index.html","w",encoding="utf-8").write(out) if __import__("os").path.isdir("/Users/artelonga/projects/co/co-web/static") else None
print("wrote shannon.html", len(out),"B")
print("p1: %d commits (%s), %d bits, peak %.0f b/s | today: %d commits %d bits"%(
  panel1["n"],panel1["by_repo"],panel1["total"],panel1["peak"],panel1["today_n"],panel1["today_bits"]))
print("p3: %d%% redundant (raw %d -> gzip %d)"%(panel3["redundancy"],raw,gz))
print("p5: MI(repo;type)=%.3f bits = %.0f%% of H(type)=%.3f"%(mi,panel5["frac"],Hy))
