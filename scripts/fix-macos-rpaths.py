#!/usr/bin/env python3
"""Supprime les LC_RPATH en double de tous les Mach-O d'un .app (dyld refuse un binaire avec un rpath dupliqué).
Usage : fix-macos-rpaths.py Aurex.app"""
import re, subprocess, sys
from collections import Counter
from pathlib import Path


def rpaths(otool_l: str):
    out, lines = [], otool_l.splitlines()
    for i, l in enumerate(lines):
        if re.match(r"\s*cmd LC_RPATH\s*$", l):
            for m in lines[i + 1 : i + 4]:
                g = re.match(r"\s*path (.+) \(offset \d+\)\s*$", m)
                if g:
                    out.append(g.group(1))
                    break
    return out


def fix(f: Path) -> int:
    r = subprocess.run(["otool", "-l", str(f)], capture_output=True, text=True)
    if r.returncode != 0:
        return 0  # pas un Mach-O
    removed = 0
    for path, n in Counter(rpaths(r.stdout)).items():
        for _ in range(n - 1):
            subprocess.run(["install_name_tool", "-delete_rpath", path, str(f)], check=True)
            removed += 1
    return removed


if __name__ == "__main__":
    app = Path(sys.argv[1])
    files = [p for p in app.rglob("*") if p.is_file() and not p.is_symlink() and (p.suffix in (".dylib", ".so") or "MacOS" in p.parts)]
    total = 0
    for f in files:
        n = fix(f)
        if n:
            print(f"{f.relative_to(app)} : {n} rpath en double supprimé(s)")
        total += n
    print(f"{len(files)} fichiers examinés, {total} doublons supprimés")
