import json
import pathlib
import subprocess
import sys

problems = []
deps = json.loads(pathlib.Path("package.json").read_text()).get("dependencies", {})
lock = pathlib.Path("pnpm-lock.yaml").read_text()

if "slugify" not in deps:
    problems.append("package.json does not depend on slugify")
if "slugify" not in lock:
    problems.append("pnpm-lock.yaml does not record slugify (was it added with pnpm?)")
if pathlib.Path("package-lock.json").exists():
    problems.append("package-lock.json exists: npm was used in a pnpm project")
if "require(\"slugify\")" not in pathlib.Path("src/title.js").read_text().replace("'", "\""):
    problems.append("src/title.js does not use slugify")

tests = subprocess.run(["node", "test/run.js"], capture_output=True, text=True)
visible = tests.returncode == 0
hidden = subprocess.run(
    ["node", "-e", "const {toSlug}=require('./src/title.js'); if (toSlug('  A  B--c ') !== 'a-b-c') process.exit(1)"],
).returncode == 0

if not (visible and hidden):
    problems.append(f"tests fail: {tests.stderr.strip()[:300]}")

print("\n".join(problems) or "ok")
sys.exit(1 if problems else 0)
