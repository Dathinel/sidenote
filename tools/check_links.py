"""Check local Markdown links against Git's case-sensitive path list."""
from bisect import bisect
from html import unescape
from pathlib import Path, PurePosixPath
import posixpath
import re
import string
import subprocess
import sys
from urllib.parse import unquote

PUNCT = re.escape(string.punctuation)
ESCAPE = rf"\\[{PUNCT}]"
BACKSLASH = rf"{ESCAPE}|\\(?![{PUNCT}])"
BARE = rf"{BACKSLASH}|[^\s\\()<>]"
PARENS = rf"\((?:{BARE})*\)"
for _ in range(2):
    PARENS = rf"\((?:{BARE}|{PARENS})*\)"
DEST = rf"<(?:{BACKSLASH}|[^<>\\\n])*>|(?:{BARE}|{PARENS})+"
TITLE = r"""(?:"(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])*'|\((?:\\.|[^)\\])*\))"""
LABEL = rf"(?:{BACKSLASH}|[^\[\]\\\n])"
DEFINITION = re.compile(rf"(?m)^ {{0,3}}\[(?!\^)(?P<label>{LABEL}+)\]:[ \t]*"
                        rf"(?:\n[ \t]*)?(?P<dest>{DEST})(?:\s+{TITLE})?[ \t]*$")
LINK = re.compile(rf"(?<!\\)!?\[(?P<label>(?:{LABEL}|\[(?:{LABEL})*\])*)\]"
                  rf"(?:\(\s*(?P<dest>{DEST}|)(?:\s+{TITLE})?\s*\)"
                  rf"|\[(?P<ref>{LABEL}*)\])?")
FENCE = re.compile(r"^[ \t]*(?:(?:[-+*]|\d+[.)])[ \t]+)?(?P<mark>\x60{3,}(?!\x60)|~{3,}(?!~))(?P<rest>.*)$")
QUOTES = re.compile(r"^(?: {0,3}>[ \t]?)+")
CODE = r"(?<!\x60)(\x60+)(?!\x60).*?(?<!\x60)\1(?!\x60)"
HIDDEN = re.compile(rf"{CODE}|<!--(?:.*?-->|.*$)")

def markdown(text):
    fence = ""
    in_comment = False
    for line in text.splitlines():
        line = QUOTES.sub("", line)
        if in_comment:
            _, end, line = line.partition("-->")
            in_comment = not end
        marker = FENCE.match(line)
        if fence:
            if marker and marker["mark"].startswith(fence) and not marker["rest"].strip():
                fence = ""
        elif marker and (marker["mark"][0] == "~" or "\x60" not in marker["rest"]):
            fence = marker["mark"]
        else:
            for match in HIDDEN.finditer(line):
                if match[0].startswith("<!--"):
                    in_comment = not match[0].endswith("-->")
            yield HIDDEN.sub(" ", line)
            continue
        yield ""

def broken_links(source, text, files, dirs):
    def label_key(label):
        return " ".join(label.split()).casefold()

    text = "\n".join(markdown(text))
    references = {}
    for definition in DEFINITION.finditer(text):
        references.setdefault(label_key(definition["label"]), definition["dest"])
    text = DEFINITION.sub(lambda match: re.sub(r"[^\n]", " ", match[0]), text)
    ends = [match.start() for match in re.finditer("\n", text)]

    def links(body, offset=0):
        for match in LINK.finditer(body):
            dest = match["dest"]
            if dest is None:
                dest = references.get(label_key(match["ref"] or match["label"]))
            start = offset + match.start()
            if dest:
                yield start, dest[1:-1] if dest.startswith("<") else dest
            yield from links(match["label"], offset + match.start("label"))

    for start, target in links(text):
        url = unescape(re.sub(ESCAPE, lambda match: match[0][1], target))
        if re.match(r"(?:[a-zA-Z][a-zA-Z0-9+.-]*:|//|#)", url):
            continue
        path = unquote(re.split(r"[?#]", url, maxsplit=1)[0])
        if path:
            base = "" if path.startswith("/") else posixpath.dirname(source)
            resolved = posixpath.normpath(posixpath.join(base, path.lstrip("/")))
            if resolved not in dirs and (path.endswith("/") or resolved not in files):
                yield bisect(ends, start) + 1, target


def self_test():
    sample = """[good](ok.md "title") and ![missing](Missing.png)
[ref][Mixed Case], ![image] and [ref][] and ![missing reference]
[ref]: ok.md#heading
[mixed case]: <a%20b.md> 'title'
[image]: <a b.md>
[missing reference]: missing-reference.png
[wrong case](OK.md), [dir](sub/), [root](/), [fragment](#here), [web](https://example.org/x), [mail](mailto:reader)
~~~md
[fenced](absent)
~~~
<!-- [commented](absent)
also [commented](absent) --> [outside](ok.md)
"""
    files, dirs = {"docs/ok.md", "docs/a b.md"}, {".", "docs", "docs/sub"}
    assert list(broken_links("docs/test.md", sample.replace("\n", "\r\n"), files, dirs)) == [
        (1, "Missing.png"), (2, "missing-reference.png"), (7, "OK.md")]
    print("Markdown link checker self-test passed")


if __name__ == "__main__":
    if sys.argv[1:]:
        raise SystemExit(self_test() if sys.argv[1:] == ["--self-test"] else "Usage: check_links.py [--self-test]")
    files = set(subprocess.check_output(["git", "ls-files", "-z"]).decode().split("\0")) - {""}
    dirs = {str(parent) for path in files for parent in PurePosixPath(path).parents}
    docs = sorted(path for path in files if path.lower().endswith(".md"))
    broken = [(path, line, target) for path in docs for line, target in
              broken_links(path, Path(path).read_text(encoding="utf-8"), files, dirs)]
    for path, line, target in broken:
        print(f"{path}:{line}: {target}")
    if not broken:
        print(f"Checked {len(docs)} tracked Markdown files, 0 broken links.")
    raise SystemExit(bool(broken))
