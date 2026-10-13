#!/usr/bin/env python3
"""Generate the third-party licence notices shown in the app's About screen (plan 5.5).

Reads the real dependency graphs, not a hand-kept list:
- Rust: `cargo metadata` for the Android targets, every crate reachable from squib-mobile
  through normal dependencies, with the licence files shipped in each published crate.
- Android: Gradle's release runtime classpath, with each library group's licence from
  an explicit table (unknown groups fail the check).

usage: third_party_notices.py [--check]
Writes apps/android/app/src/main/assets/third_party_notices.json; --check exits 1 if the
committed file is out of date (or a licence is unrecognised) instead of writing it.
"""
import hashlib, json, re, subprocess, sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "apps/android/app/src/main/assets/third_party_notices.json"
TARGETS = ["aarch64-linux-android", "x86_64-linux-android"]
LICENSE_FILE = re.compile(r"^(licen[cs]e|copying|notice|copyright|unlicense)([-._].*)?$", re.I)

# Licences of the Android library groups on the release classpath. Gradle resolves these
# through module metadata, which carries no licence, so the table is explicit; a library
# from a group not listed here fails --check and needs a review before it ships.
GROUP_LICENSES = [
    (re.compile(r"^androidx\."), "Apache-2.0"),
    (re.compile(r"^org\.jetbrains\.kotlin(x)?$"), "Apache-2.0"),
    (re.compile(r"^org\.jetbrains$"), "Apache-2.0"),  # annotations
    (re.compile(r"^com\.google\.guava$"), "Apache-2.0"),  # listenablefuture
    (re.compile(r"^org\.jspecify$"), "Apache-2.0"),
    # Offered under either licence; Squib uses it under Apache-2.0.
    (re.compile(r"^net\.java\.dev\.jna$"), "LGPL-2.1-or-later OR Apache-2.0"),
]


def text_id(text):
    return hashlib.sha256(text.encode()).hexdigest()[:12]


def rust_components(texts):
    pkgs, reach = {}, set()
    for target in TARGETS:
        meta = json.loads(subprocess.run(
            ["cargo", "metadata", "--format-version", "1", "--locked", "--offline", "--filter-platform", target],
            cwd=ROOT, check=True, capture_output=True, text=True).stdout)
        pkgs.update({p["id"]: p for p in meta["packages"]})
        nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
        start = next(p["id"] for p in meta["packages"] if p["name"] == "squib-mobile")
        stack = [start]
        while stack:
            i = stack.pop()
            if i in reach:
                continue
            reach.add(i)
            for d in nodes[i]["deps"]:
                if any(k["kind"] is None for k in d["dep_kinds"]):
                    stack.append(d["pkg"])
    members = {p for p in pkgs if pkgs[p]["source"] is None}
    out = []
    for i in sorted(reach - members, key=lambda i: (pkgs[i]["name"], pkgs[i]["version"])):
        p = pkgs[i]
        d = Path(p["manifest_path"]).parent
        ids = []
        for f in sorted(d.iterdir()):
            if f.is_file() and LICENSE_FILE.match(f.name):
                t = f.read_text(errors="replace").strip()
                texts[text_id(t)] = t
                ids.append(text_id(t))
        upstream = ROOT / "scripts/licenses/upstream" / f"{p['name']}.txt"
        if not ids and upstream.is_file():
            # The published crate omits its licence file; this is the text from the
            # crate's own source repository (see scripts/licenses/upstream/README.md).
            t = upstream.read_text().strip()
            texts[text_id(t)] = t
            ids.append(text_id(t))
        if not ids:
            # No licence file in the published crate: keep its own copyright header if the
            # source has one, and attach the licence text its SPDX expression names.
            header = copyright_header(d / "src/lib.rs")
            if header:
                texts[text_id(header)] = header
                ids.append(text_id(header))
            lic = p["license"] or ""
            if "MPL-2.0" in lic:
                ids.append(canonical(texts, "MPL-2.0"))
            elif "Apache-2.0" in lic:
                ids.append("apache")  # resolved to the in-graph Apache text below
        out.append({"ecosystem": "rust", "name": p["name"], "version": p["version"],
                    "license": p["license"] or "unspecified", "texts": sorted(set(ids))})
    return out


def copyright_header(path):
    """The leading comment block of a source file, if it states a copyright."""
    if not path.is_file():
        return None
    lines = []
    for line in path.read_text(errors="replace").splitlines():
        t = line.strip()
        if t.startswith(("/*", "*", "//", "*/")) or (not t and lines):
            lines.append(re.sub(r"^(/\*+|\*/|\*|//[!/]?)\s?", "", t))
            if t.endswith("*/"):
                break
        else:
            break
    text = "\n".join(lines).strip()
    return text if re.search(r"copyright", text, re.I) else None


def canonical(texts, spdx):
    t = (ROOT / "scripts/licenses" / f"{spdx}.txt").read_text().strip()
    texts[text_id(t)] = t
    return text_id(t)


def android_components(texts, apache_text_id):
    raw = subprocess.run(["./gradlew", "-q", ":app:dependencies", "--configuration", "releaseRuntimeClasspath"],
                         cwd=ROOT / "apps/android", check=True, capture_output=True, text=True).stdout
    coords = set()
    for line in raw.splitlines():
        m = re.match(r"^[|+\\\- ]+([\w.\-]+):([\w.\-]+):(\S+)(?: -> (\S+))?(.*)$", line)
        if not m or "(c)" in m.group(5) or "(n)" in m.group(5):
            continue
        coords.add((m.group(1), m.group(2), m.group(4) or m.group(3)))
    out, unknown = [], []
    for g, a, v in sorted(coords):
        lic = next((l for r, l in GROUP_LICENSES if r.match(g)), None)
        if lic is None:
            unknown.append(f"{g}:{a}:{v}")
            lic = "unknown"
        out.append({"ecosystem": "android", "name": f"{g}:{a}", "version": v, "license": lic,
                    "texts": [apache_text_id] if "Apache-2.0" in lic else []})
    return out, unknown


def main():
    texts = {}
    rust = rust_components(texts)
    serde = next(c for c in rust if c["name"] == "serde")
    apache = next(t for t in serde["texts"] if texts[t].startswith("Apache License") or "Apache License" in texts[t][:200])
    for c in rust:
        c["texts"] = sorted({apache if t == "apache" else t for t in c["texts"]})
    android, unknown = android_components(texts, apache)
    comps = rust + android
    missing = [f"{c['name']} {c['version']}" for c in comps if not c["texts"]]
    used = {t for c in comps for t in c["texts"]}
    doc = {
        "generated_by": "scripts/third_party_notices.py",
        "note": "Licence files are those shipped in each published Rust crate; Android libraries are listed "
                "with their group's licence. JNA is used under Apache-2.0 (it is also offered under LGPL-2.1-or-later). "
                "SQLite, bundled through libsqlite3-sys, is in the public domain.",
        "components": comps,
        "texts": {k: texts[k] for k in sorted(used)},
    }
    body = json.dumps(doc, indent=1, sort_keys=True, ensure_ascii=False) + "\n"
    problems = [f"unrecognised licence: {u}" for u in unknown] + [f"no licence text: {m}" for m in missing]
    for p in problems:
        print(p, file=sys.stderr)
    if "--check" in sys.argv:
        current = OUT.read_text() if OUT.exists() else ""
        if current != body:
            print(f"{OUT.relative_to(ROOT)} is out of date: run scripts/third_party_notices.py", file=sys.stderr)
            return 1
        return 1 if problems else 0
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text(body)
    print(f"{len(rust)} Rust crates, {len(android)} Android libraries, {len(doc['texts'])} distinct licence texts, "
          f"{len(body) // 1024} KB")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
