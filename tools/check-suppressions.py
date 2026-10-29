import datetime
import pathlib
import re

text = pathlib.Path("deny.toml").read_text()
today = datetime.date.today()
stale = []
undated = []
for match in re.finditer(r'\{\s*id = "(RUSTSEC-[^"]+)"(.*?)\}', text, re.S):
    identifier, body = match.group(1), match.group(2)
    stamp = re.search(r"by (\d{4}-\d{2}-\d{2})", body)
    if stamp is None:
        undated.append(identifier)
        continue
    if datetime.date.fromisoformat(stamp.group(1)) < today:
        stale.append(identifier)
if undated:
    raise SystemExit(
        "no recheck date in the reason, so nobody is told when to look again: "
        + ", ".join(undated)
    )
if stale:
    raise SystemExit(
        "past the recheck date, so the suppression must be decided again: "
        + ", ".join(stale)
    )
