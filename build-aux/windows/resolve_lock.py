#!/usr/bin/env python3
"""Resolve the missing Cargo.lock entries for the Windows port.

Uses the crates.io API to resolve the dependency closure of the new
Windows-only dependencies (windows, embed-resource) and prints the
[[package]] blocks that must be added to Cargo.lock, preferring versions
already present in the lock (like cargo does).
"""
import json
import re
import sys
import urllib.request

UA = {"User-Agent": "tsukimi-windows-port (lock resolver)"}
LOCK = r"C:\Users\Administrator\Desktop\tsukimi-main\tsukimi-main\Cargo.lock"


def api(path):
    req = urllib.request.Request(f"https://crates.io/api/v1/{path}", headers=UA)
    with urllib.request.urlopen(req, timeout=30) as r:
        return json.load(r)


def parse_req(req):
    """Return (min_version, upper_bound) tuples; either may be None (unbounded)."""
    req = req.strip()
    if req in ("*", ""):
        return (0, 0, 0), None
    m = re.match(r"^(>=|<=|>|<|\^|~|=)?\s*(\d+)(?:\.(\d+))?(?:\.(\d+))?(?:[+-].*)?$", req)
    if not m:
        raise ValueError(f"unhandled req: {req}")
    op = m.group(1) or "^"
    nums = tuple(int(m.group(i) or 0) for i in (2, 3, 4))
    if op == ">=":
        return nums, None
    if op == "<":
        return (0, 0, 0), nums
    if op == "<=":
        return (0, 0, 0), tuple(list(nums[:-1]) + [nums[2] + 1])
    if op == ">":
        return nums, None
    if op in ("=",):
        return nums, nums
    # caret semantics
    if op == "^":
        if nums[0] > 0:
            return nums, (nums[0] + 1, 0, 0)
        if nums[1] > 0:
            return nums, (0, nums[1] + 1, 0)
        return nums, (0, 0, nums[2] + 1)
    # tilde semantics
    if nums[1] > 0 or nums[2] > 0:
        return nums, (nums[0], nums[1] + 1, 0)
    return nums, (nums[0] + 1, 0, 0)


def vtuple(version):
    return tuple(int(x) for x in re.match(r"^(\d+)\.(\d+)\.(\d+)", version).groups())


def satisfies(version, req):
    """Mirror the semver crate's comparator semantics exactly.

    Partial versions in explicit comparators (e.g. `<=0.61`) keep the
    unspecified components as wildcards, so `<=0.61` matches 0.61.2.
    """
    v = vtuple(version)
    for part in req.split(","):
        part = part.strip()
        if part in ("*", ""):
            continue
        m = re.match(r"^(>=|<=|>|<|\^|~|=)?\s*(\d+)(?:\.(\d+))?(?:\.(\d+))?(?:[+-].*)?$", part)
        if not m:
            raise ValueError(f"unhandled req: {part}")
        op = m.group(1) or "^"
        major = int(m.group(2))
        minor = int(m.group(3)) if m.group(3) is not None else None
        patch = int(m.group(4)) if m.group(4) is not None else None

        def exact():
            if v[0] != major:
                return False
            if minor is not None and v[1] != minor:
                return False
            if patch is not None and v[2] != patch:
                return False
            return True

        def greater():
            if v[0] != major:
                return v[0] > major
            if minor is None:
                return False
            if v[1] != minor:
                return v[1] > minor
            if patch is None:
                return False
            return v[2] > patch

        def less():
            if v[0] != major:
                return v[0] < major
            if minor is None:
                return False
            if v[1] != minor:
                return v[1] < minor
            if patch is None:
                return False
            return v[2] < patch

        if op in (">=",):
            ok = exact() or greater()
        elif op == ">":
            ok = greater()
        elif op == "<=":
            ok = exact() or less()
        elif op == "<":
            ok = less()
        elif op == "=":
            ok = exact()
        elif op == "^":
            if v[0] != major:
                ok = False
            elif minor is None:
                ok = True
            elif patch is None:
                ok = v[1] >= minor if major > 0 else v[1] == minor
            elif major > 0:
                ok = v[1] > minor or (v[1] == minor and v[2] >= patch)
            elif minor > 0:
                ok = v[1] == minor and v[2] >= patch
            else:
                ok = v[1] == 0 and v[2] == patch
        elif op == "~":
            if v[0] != major:
                ok = False
            elif minor is not None and v[1] != minor:
                ok = False
            elif patch is not None and v[2] < patch:
                ok = False
            else:
                ok = True
        else:
            raise ValueError(f"unhandled op: {op}")
        if not ok:
            return False
    return True


def versions_of(name):
    """Non-yanked versions of a crate -> {num: None} (checksums fetched lazily)."""
    d = api(f"crates/{name}")
    return {v["num"]: None for v in d["versions"] if not v["yanked"]}


def checksum_of(name, version):
    d = api(f"crates/{name}/{version}")
    return d["version"]["checksum"]


def deps_of(name, version):
    d = api(f"crates/{name}/{version}/dependencies")
    out = []
    for dep in d["dependencies"]:
        if dep["kind"] not in ("normal", "build"):
            continue  # dev-deps of external crates are not in the lock
        if dep["optional"]:
            continue  # optional deps are only pulled via features we don't enable
        out.append((dep["crate_id"], dep["req"], dep["target"], dep["kind"]))
    return out


def main():
    lock_text = open(LOCK, encoding="utf-8").read()
    locked = {}
    for m in re.finditer(r'\[\[package\]\]\nname = "([^"]+)"\nversion = "([^"]+)"', lock_text):
        locked.setdefault(m.group(1), []).append(m.group(2))

    resolved = {}  # name -> (version, checksum, [dep names])

    def resolve(name, req):
        if name in resolved:
            assert satisfies(resolved[name][0], req), f"{name} {resolved[name][0]} !~ {req}"
            return
        # Prefer a version already in the lock (cargo preserves locked versions).
        candidates = [v for v in locked.get(name, []) if satisfies(v, req)]
        if not candidates:
            candidates = [v for v in versions_of(name) if satisfies(v, req)]
            if not candidates:
                raise SystemExit(f"no version of {name} satisfies {req}")
        # max matching
        def key(v):
            return tuple(int(x) for x in re.match(r"^(\d+)\.(\d+)\.(\d+)", v).groups())
        version = max(candidates, key=key)
        checksum = checksum_of(name, version)
        dep_list = deps_of(name, version)
        resolved[name] = (version, checksum, dep_list)
        for dep_name, dep_req, _target, _kind in dep_list:
            resolve(dep_name, dep_req)

    for root, req in [("windows", "^0.62"), ("embed-resource", "^3.0")]:
        resolve(root, req)

    print(f"# {len(resolved)} packages in closure")
    for name in sorted(resolved):
        version, checksum, dep_list = resolved[name]
        in_lock = name in locked and version in locked[name]
        print(f"{'LOCKED' if in_lock else 'NEW   '} {name} {version}")
        for dep_name, dep_req, target, kind in dep_list:
            marker = ""
            if target:
                marker += f" [target={target}]"
            if kind == "build":
                marker += " [build]"
            print(f"         -> {dep_name} {dep_req}{marker}")
        if not in_lock:
            print(f"         checksum = {checksum}")


if __name__ == "__main__":
    main()
