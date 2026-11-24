#!/usr/bin/env python3
"""List everything OBSBOT's SDK offers: every function in the headers with its device category, and every
function the library exports that the headers leave out, with its parameter types (from the symbol table).

    python3 inventory.py [path/to/libdev_v2.1.0_8] > inventory.txt

Reads the local SDK only; sends nothing to the camera. Needs `nm` (binutils).
"""
import pathlib
import re
import subprocess
import sys

sdk = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else pathlib.Path.home() / "obsbot-sdk/libdev_v2.1.0_8")
headers = "".join(p.read_text(errors="replace") for p in sorted((sdk / "include").rglob("*.hpp")))
lib = next((p for p in [sdk / "linux/x86_64-release/libdev.so", *sorted((sdk / "linux/x86_64-release").glob("libdev.so*"))] if p.exists()), None)

doc = re.compile(r"/\*\*(.*?)\*/\s*((?:virtual\s+|static\s+)?[\w:<>,\s\*&]+?[\s\*&](\w+)\s*\(([^;{]*)\)\s*(?:const)?\s*;)", re.S)
declared = {}
for m in doc.finditer(headers):
    text, name, args = m.group(1), m.group(3), " ".join(m.group(4).split())
    category = re.search(r"@category\s+(.*)", text)
    brief = re.search(r"@brief\s+(.*)", text)
    declared[name] = (category.group(1).strip() if category else "(no category)", args, brief.group(1).strip() if brief else "")
for m in re.finditer(r"^\s+(?:virtual\s+|static\s+)?[\w:<>,\s\*&]+?[\s\*&](\w+)\s*\(([^;{]*)\)\s*(?:const)?\s*;", headers, re.M):
    declared.setdefault(m.group(1), ("(no doc)", " ".join(m.group(2).split()), ""))

tiny = {n: v for n, v in declared.items() if re.search(r"tiny2|tiny 2|\ball\b", v[0], re.I)}
print(f"SDK: {sdk}")
print(f"Declared in the headers: {len(declared)}; for the Tiny 2 series or all devices: {len(tiny)}")
print("\n== For the Tiny 2 series or all devices ==")
for name, (category, args, brief) in sorted(tiny.items()):
    print(f"{name}({args})\n    [{category}] {brief}")
print("\n== No category or no documentation ==")
for name, (category, args, brief) in sorted(declared.items()):
    if category in ("(no category)", "(no doc)"):
        print(f"{name}({args})  {brief}")
print("\n== Other devices only ==")
for name, (category, args, brief) in sorted(declared.items()):
    if name not in tiny and category not in ("(no category)", "(no doc)"):
        print(f"{name}  [{category}]")

if lib:
    symbols = subprocess.run(["nm", "-DC", str(lib)], capture_output=True, text=True).stdout
    exported = {}
    for line in symbols.splitlines():
        m = re.match(r"[0-9a-f]+ T (Device|Devices|DevUpgrade)::(\w+)\((.*)\)", line)
        if m:
            exported.setdefault((m.group(1), m.group(2)), m.group(3))
    hidden = {k: v for k, v in exported.items() if k[1] not in declared}
    print(f"\n== In the library but not in the headers: {len(hidden)} ({lib.name}) ==")
    for (cls, name), args in sorted(hidden.items()):
        print(f"{cls}::{name}({args})")
else:
    print("\n(No Linux x86_64 library found, so the hidden functions are not listed.)")
