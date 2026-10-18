# python3 tcclog.py <seconds> <needle>: TCC decisions for the Documents folder whose attribution mentions <needle>
import re, subprocess, sys
secs, needle = sys.argv[1], sys.argv[2]
out = subprocess.run(["/usr/bin/log", "show", "--last", f"{secs}s", "--predicate", 'subsystem == "com.apple.TCC"', "--style", "compact"],
                     capture_output=True, text=True).stdout
by = {}
for line in out.splitlines():
    m = re.search(r"msgID=([\d.]+)", line)
    if not m: continue
    d = by.setdefault(m.group(1), {})
    if "AUTHREQ_CTX" in line:
        d["service"] = re.search(r"service=(\w+)", line).group(1); d["time"] = line[11:23]
    if "AUTHREQ_ATTRIBUTION" in line: d["attr"] = line
    if "AUTHREQ_RESULT" in line: d["res"] = re.search(r"authValue=(\d)", line).group(1)
for k, d in by.items():
    if needle in d.get("attr", "") and "Documents" in d.get("service", ""):
        paths = re.findall(r"binary_path=([^,}]+)", d["attr"])
        print(d["time"], d["service"], "authValue(2=allow,0=deny)=", d.get("res"), "paths:", paths)
