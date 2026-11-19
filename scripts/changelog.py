#!/usr/bin/env python3
"""Keep CHANGELOG.md about released versions.

Release candidates are throwaway: `0.1.2-rc.3` is not something anyone installs on purpose
a month later, and a section per candidate buries the release they were rehearsing for. So
candidates never get a section of their own; the entry stays under `## Unreleased` until the
real version ships, and anything a candidate did claim is folded back.

A pull request does not edit CHANGELOG.md: it adds its bullets as a file of their own under
changelog.d/, so two open pull requests never conflict over the top of `## Unreleased`.

    changelog.py collect           # changelog.d/*.md -> ## Unreleased, then the files go
    changelog.py promote 0.1.2     # collect, then Unreleased + any 0.1.2-rc.* -> ## 0.1.2 — date
    changelog.py tidy              # fold existing rc sections into their release

All are idempotent, and all leave the wording alone: bullets are moved, never rewritten.
"""

import datetime
import pathlib
import re
import sys

PATH = pathlib.Path(__file__).resolve().parent.parent / "CHANGELOG.md"
FRAGMENTS = PATH.parent / "changelog.d"
HEADING = re.compile(r"^## (?P<title>.+?)(?: — (?P<date>[\d-]+))?$", re.MULTILINE)


class Section:
    def __init__(self, title: str, date: str | None, body: str):
        self.title = title
        self.date = date
        self.body = body.strip("\n")

    @property
    def version(self) -> str | None:
        """The release this section belongs to: `0.1.2-rc.3` and `0.1.2` both give `0.1.2`."""
        return self.title.split("-rc.")[0] if self.title != "Unreleased" else None

    @property
    def is_candidate(self) -> bool:
        return "-rc." in self.title

    def render(self) -> str:
        dated = f"## {self.title} — {self.date}" if self.date else f"## {self.title}"
        return f"{dated}\n\n{self.body}\n" if self.body else f"{dated}\n"


def read() -> tuple[str, list[Section]]:
    text = PATH.read_text()
    marks = list(HEADING.finditer(text))
    preamble = text[: marks[0].start()] if marks else text
    sections = []
    for i, m in enumerate(marks):
        end = marks[i + 1].start() if i + 1 < len(marks) else len(text)
        sections.append(Section(m["title"], m["date"], text[m.end() : end]))
    return preamble, sections


def write(preamble: str, sections: list[Section]) -> None:
    PATH.write_text(preamble.rstrip("\n") + "\n\n" + "\n".join(s.render() for s in sections))


def bullets(body: str) -> list[str]:
    """Split a body into top-level bullets, each keeping its continuation lines."""
    out: list[str] = []
    for line in body.splitlines():
        if line.startswith("- ") or not out:
            out.append(line)
        else:
            out[-1] += "\n" + line
    return [b for b in out if b.strip()]


def merge(bodies: list[str]) -> str:
    """Concatenate bodies, dropping bullets that already say the same thing."""
    seen: list[str] = []
    for body in bodies:
        for bullet in bullets(body):
            if bullet not in seen:
                seen.append(bullet)
    return "\n".join(seen)


def fragments() -> list[pathlib.Path]:
    if not FRAGMENTS.is_dir():
        return []
    return sorted(p for p in FRAGMENTS.glob("*.md") if p.name != "README.md")


def collect() -> None:
    files = fragments()
    if not files:
        print("collected nothing: changelog.d has no fragments")
        return
    preamble, sections = read()
    unreleased = next((s for s in sections if s.title == "Unreleased"), None)
    body = merge([unreleased.body if unreleased else ""] + [p.read_text() for p in files])
    if unreleased:
        unreleased.body = body
    else:
        sections.insert(0, Section("Unreleased", None, body))
    write(preamble, sections)
    for p in files:
        p.unlink()
    print(f"collected {len(files)} fragment(s) into ## Unreleased")


def promote(version: str) -> None:
    collect()
    preamble, sections = read()
    today = f"{datetime.date.today():%Y-%m-%d}"

    fold = [s for s in reversed(sections) if s.is_candidate and s.version == version]
    fold += [s for s in sections if s.title == "Unreleased"]
    body = merge([s.body for s in fold])

    kept = [s for s in sections if s not in fold]
    existing = next((s for s in kept if s.title == version), None)
    if existing:  # a re-run, or a release prepared twice
        existing.body = merge([existing.body, body])
        existing.date = today
    else:
        kept.insert(0, Section(version, today, body or "- No user-visible changes."))
    write(preamble, kept)
    print(f"promoted {len(fold)} section(s) into ## {version} — {today}")


def tidy() -> None:
    preamble, sections = read()
    releases = {s.title for s in sections if not s.is_candidate and s.title != "Unreleased"}

    kept: list[Section] = []
    for s in sections:
        if not s.is_candidate:
            kept.append(s)
            continue
        if s.version in releases:  # fold into the release it rehearsed
            target = next(k for k in kept if k.title == s.version)
            target.body = merge([s.body, target.body])
        else:  # never released: it is still what is coming next
            unreleased = next((k for k in kept if k.title == "Unreleased"), None)
            if unreleased:
                unreleased.body = merge([s.body, unreleased.body])
            else:
                kept.insert(0, Section("Unreleased", None, s.body))
    write(preamble, kept)
    print("tidied")


if __name__ == "__main__":
    match sys.argv[1:]:
        case ["collect"]:
            collect()
        case ["promote", version]:
            promote(version)
        case ["tidy"]:
            tidy()
        case _:
            sys.exit(__doc__)
