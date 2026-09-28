"""Offline stand-in for npm and pnpm: logs each call, installs a fake slugify, runs tests."""

import json
import os
import pathlib
import subprocess
import sys

VERSIONS = {"npm": "10.8.0", "pnpm": "9.12.0"}
INSTALL = {"npm": {"install", "i", "add"}, "pnpm": {"add", "install", "i"}}
SLUGIFY = """"use strict"
module.exports = function slugify(text, options) {
  const lower = typeof options === "object" && options !== null && options.lower
  const slug = String(text).trim().replace(/[^A-Za-z0-9]+/g, "-").replace(/^-+|-+$/g, "")
  return lower ? slug.toLowerCase() : slug
}
"""
KNOWN = {"slugify": "1.6.6"}


def log(tool, args):
    path = os.environ.get("BENCH_LOG")

    if path:
        with open(path, "a") as handle:
            handle.write(json.dumps([tool, *args]) + "\n")


def split(spec):
    if spec.startswith("@") or "@" not in spec:
        return spec, None

    name, version = spec.rsplit("@", 1)

    return name, version


def install(tool, packages):
    manifest_path = pathlib.Path("package.json")
    manifest = json.loads(manifest_path.read_text())
    deps = manifest.setdefault("dependencies", {})

    for spec in packages:
        name, version = split(spec)

        if name not in KNOWN:
            print(f"{tool} ERR! 404 Not Found - {name}", file=sys.stderr)
            return 1

        deps[name] = version or f"^{KNOWN[name]}"

    for name in deps:
        target = pathlib.Path("node_modules", name)
        target.mkdir(parents=True, exist_ok=True)
        (target / "index.js").write_text(SLUGIFY)
        (target / "package.json").write_text(json.dumps({"name": name, "version": KNOWN[name]}))

    manifest_path.write_text(json.dumps(manifest, indent=2) + "\n")
    write_lock(tool, deps)
    print(f"added {len(deps)} package(s) with {tool}")

    return 0


def write_lock(tool, deps):
    if tool == "npm":
        packages = {f"node_modules/{name}": {"version": KNOWN[name]} for name in deps}
        lock = {"name": "titles", "lockfileVersion": 3, "packages": packages}
        pathlib.Path("package-lock.json").write_text(json.dumps(lock, indent=2) + "\n")
        return

    lines = ["lockfileVersion: '9.0'", "", "importers:", "", "  .:", "    dependencies:"]

    for name, spec in deps.items():
        lines += [f"      {name}:", f"        specifier: {spec}", f"        version: {KNOWN[name]}"]

    pathlib.Path("pnpm-lock.yaml").write_text("\n".join(lines) + "\n")


def main(tool):
    args = sys.argv[1:]
    log(tool, args)
    words = [arg for arg in args if not arg.startswith("-")]

    if args[:1] in (["--version"], ["-v"]):
        print(VERSIONS[tool])
        return 0
    if words[:1] == ["test"] or words[:2] == ["run", "test"] or words[:1] == ["t"]:
        return subprocess.run(["node", "test/run.js"]).returncode
    if words[:1] and words[0] in INSTALL[tool]:
        return install(tool, words[1:])

    print(f"{tool}: unsupported command in this environment: {' '.join(args)}", file=sys.stderr)

    return 1
