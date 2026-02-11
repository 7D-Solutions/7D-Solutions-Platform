#!/usr/bin/env python3
import os, time, statistics, requests
BASE=os.environ.get("BASE","http://localhost:8080")
TENANT=os.environ.get("TENANT_ID","")
EMAIL=os.environ.get("EMAIL","")
PWD=os.environ.get("PASSWORD","")
N=int(os.environ.get("N","200"))

def pct(lst,p):
    lst=sorted(lst)
    k=int(round((p/100.0)*(len(lst)-1)))
    return lst[k]

def main():
    lat=[]
    for i in range(N):
        t0=time.time()
        r=requests.post(f"{BASE}/api/auth/login", json={"tenant_id":TENANT,"email":EMAIL,"password":PWD})
        dt=(time.time()-t0)*1000
        lat.append(dt)
        if r.status_code!=200:
            print("non-200", r.status_code, r.text[:200])
        time.sleep(0.01)
    print("N=",N,"mean(ms)=",round(statistics.mean(lat),2),"p95=",round(pct(lat,95),2),"p99=",round(pct(lat,99),2))

if __name__=="__main__":
    main()
