#!/usr/bin/env python3
"""Insert the new Windows-only packages into Cargo.lock.

Adds the [[package]] blocks for the dependency closure of the new
Windows-only dependencies (`windows`, `embed-resource`) resolved by
resolve_lock.py, wires them into the `tsukimi` package entry, and
re-emits the lock in cargo's canonical format (sorted by name then
semver version).
"""
import re

LOCK = r"C:\Users\Administrator\Desktop\tsukimi-main\tsukimi-main\Cargo.lock"
SRC = "registry+https://github.com/rust-lang/crates.io-index"

NEW_PACKAGES = [
    ("embed-resource", "3.0.12",
     "6168487b5e7eb2673e6efe1200b1848016e436a6ca0b8031bf04bd9d493a249f",
     ["cc", "memchr", "rustc_version", "toml", "vswhom", "winreg"]),
    ("vswhom", "0.1.0",
     "be979b7f07507105799e854203b470ff7c78a1639e330a58f183b5fea574608b",
     ["libc", "vswhom-sys"]),
    ("vswhom-sys", "0.1.3",
     "fb067e4cbd1ff067d1df46c9194b5de0e98efd2810bbc95c5d5e5f25a3231150",
     ["cc", "libc"]),
    ("windows", "0.62.2",
     "527fadee13e0c05939a6a05d5bd6eec6cd2e3dbd648b9f8e447c6518133d8580",
     ["windows-collections", "windows-core", "windows-future", "windows-numerics"]),
    ("windows-collections", "0.3.2",
     "23b2d95af1a8a14a3c7367e1ed4fc9c20e0a26e79551b1454d72583c97cc6610",
     ["windows-core"]),
    ("windows-future", "0.3.2",
     "e1d6f90251fe18a279739e78025bd6ddc52a7e22f921070ccdc67dde84c605cb",
     ["windows-core", "windows-link", "windows-threading"]),
    ("windows-numerics", "0.3.1",
     "6e2e40844ac143cdb44aead537bbf727de9b044e107a0f1220392177d15b0f26",
     ["windows-core", "windows-link"]),
    ("windows-threading", "0.2.1",
     "3949bd5b99cafdf1c7ca86b43ca564028dfe27d66958f2470940f73d86d75b37",
     ["windows-link"]),
    ("winreg", "0.56.0",
     "7d6f32a0ff4a9f6f01231eb2059cc85479330739333e0e58cadf03b6af2cca10",
     ["cfg-if", "windows-sys 0.61.2"]),
]

# Direct dependencies the tsukimi package gains on Windows.
TSUKIMI_NEW_DEPS = ["embed-resource", "gdk-pixbuf", "windows"]


def vkey(version):
    m = re.match(r"^(\d+)\.(\d+)\.(\d+)", version)
    return tuple(int(x) for x in m.groups())


def main():
    text = open(LOCK, encoding="utf-8").read()
    header, _, rest = text.partition("version = 4\n")
    blocks = [b for b in rest.strip("\n").split("\n\n") if b.strip()]

    parsed = []
    for block in blocks:
        name = re.search(r'name = "([^"]+)"', block).group(1)
        version = re.search(r'version = "([^"]+)"', block).group(1)
        parsed.append((name, version, block))

    existing = {(n, v) for n, v, _ in parsed}
    for name, version, _checksum, _deps in NEW_PACKAGES:
        assert (name, version) not in existing, f"{name} {version} already in lock"

    # Which package names have multiple versions in the lock (need
    # "name version" disambiguation in dependency lists)?
    versions_by_name = {}
    for name, version, _ in parsed:
        versions_by_name.setdefault(name, set()).add(version)
    for name, version, _c, _d in NEW_PACKAGES:
        versions_by_name.setdefault(name, set()).add(version)

    def dep_str(dep):
        if " " in dep:  # already disambiguated
            return f' "{dep}"'
        name = dep
        if len(versions_by_name.get(name, ())) > 1:
            raise SystemExit(f"dependency {name} is ambiguous; specify version")
        return f' "{name}"'

    for name, version, checksum, deps in NEW_PACKAGES:
        lines = [
            "[[package]]",
            f'name = "{name}"',
            f'version = "{version}"',
            f'source = "{SRC}"',
            f'checksum = "{checksum}"',
        ]
        if deps:
            lines.append("dependencies = [")
            for dep in sorted(deps):
                lines.append(dep_str(dep) + ",")
            lines.append("]")
        parsed.append((name, version, "\n".join(lines)))

    # Wire the new direct dependencies into the tsukimi package entry.
    for i, (name, version, block) in enumerate(parsed):
        if name != "tsukimi":
            continue
        m = re.search(r"dependencies = \[\n((?: .*\n)*)\]", block)
        assert m, "tsukimi entry has no dependencies list"
        deps = [line.strip().strip(",").strip('"') for line in m.group(1).split("\n") if line.strip()]
        for dep in TSUKIMI_NEW_DEPS:
            if dep not in deps:
                deps.append(dep)
        deps.sort()
        new_list = "dependencies = [\n" + "".join(f' "{d}",\n' for d in deps) + "]"
        parsed[i] = (name, version, block[: m.start()] + new_list + block[m.end():])

    parsed.sort(key=lambda item: (item[0], vkey(item[1])))
    out = header + "version = 4\n\n" + "\n\n".join(b for _, _, b in parsed) + "\n"
    open(LOCK, "w", encoding="utf-8", newline="\n").write(out)
    print(f"wrote {LOCK} with {len(parsed)} packages")


if __name__ == "__main__":
    main()
