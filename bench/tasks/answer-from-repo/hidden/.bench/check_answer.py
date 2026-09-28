import re
import sys
from pathlib import Path

answer = Path("ANSWER.md")
if not answer.is_file():
    sys.exit("ANSWER.md is missing")
text = answer.read_text(encoding="utf-8")
if not re.search(r"(?<!\d)8443(?!\d)", text):
    sys.exit("ANSWER.md does not name port 8443")
if "overrides.yaml" not in text:
    sys.exit("ANSWER.md does not name config/overrides.yaml")
if re.search(r"(?<!\d)7000(?!\d)", text):
    sys.exit("ANSWER.md mentions the commented-out port 7000")
print("ok")
