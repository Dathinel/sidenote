"""Check Lane U's headless render trees (render_ui.ps1 -Dev3 -Trees)."""
import json
import sys
from pathlib import Path


def nodes(node):
    yield node
    for child in node.get("children", []):
        yield from nodes(child)


def check(root):
    checked = 0
    message = "Ask works with Claude Code, Codex or OpenCode. AI notes need Claude Code."
    for width in (1320, 560):
        for theme in ("light", "dark"):
            def load(scene):
                nonlocal checked
                tree = json.loads((root / f"{scene}-{width}-{theme}.json").read_text())
                checked += 1
                elements = tree["elements"]
                keyed = {e["key"]: e for e in elements if "key" in e}
                text = [e["text"] for e in elements if "text" in e]
                accessible = {n["key"]: n for n in nodes(tree["accessibility"]) if "key" in n}
                return elements, keyed, text, accessible

            _, keyed, text, _ = load("notes-today-first-open")
            assert "today-ai-notes" in keyed and "ask-open" in keyed
            assert "ask-recheck" not in keyed and message not in text
            assert text.count("Check again") == 1
            assert "today-notes-install" in keyed
            assert not any("Copy sign-in" in t for t in text)

            _, keyed, text, _ = load("notes-click-ask")
            assert "today-ai-notes" in keyed and "ask-recheck" in keyed
            assert text.count(message) == 1

            elements, keyed, text, accessible = load("notes-settings-none")
            for key in ("notes-change-model", "notes-use-0", "notes-test-0", "notes-copy-command"):
                assert key not in keyed, key
            assert "notes-install-page" in keyed
            assert text.count("Selected") == 1
            assert "disabled" in accessible["notes-auto"]["states"]
            assert "Only applies when AI notes are on." in text
            for index in (1, 2):
                card = keyed[f"notes-helper-{index}"]["path"] + "/"
                content = [e["text"] for e in elements if e["path"].startswith(card) and "text" in e]
                assert content == ["Codex" if index == 1 else "OpenCode", "Coming in a later version"]

            _, keyed, text, accessible = load("notes-settings-on")
            assert keyed["notes-use-0"]["text"] == "In use"
            assert "click" not in accessible["notes-use-0"].get("actions", [])
            assert "Copy sign-in command: claude" in text

            for helper in ("codex", "opencode"):
                _, _, text, _ = load(f"notes-settings-saved-{helper}")
                assert text.count("Saved choice not available") == 1
                assert sum("AI notes are off" in t for t in text) == 1

            _, _, text, _ = load("notes-meeting-done")
            assert any("in recording" in t for t in text)
            assert "Your notes" not in text

            _, keyed, text, _ = load("features-settings")
            assert "nav-tasks" in keyed
            switches = [e for e in keyed.values() if e.get("role") == "switch"]
            assert text.count("On") + text.count("Off") == len(switches)

            for scene in ("ask-no-agent", "ask-no-agent-again"):
                _, _, text, _ = load(scene)
                assert message in text
                if scene.endswith("again"):
                    assert any("press Check again, or restart Sidenote if it still fails" in t for t in text)

    for name in ("listener-error", "listener-error-narrow"):
        for theme in ("light", "dark"):
            tree = json.loads((root / f"{name}-{theme}.json").read_text())
            checked += 1
            text = [e.get("text") for e in tree["elements"]]
            assert ("Off" if "narrow" in name else "Recording off") in text
    print(f"Passed Lane U checks across {checked} render trees.")


if __name__ == "__main__":
    check(Path(sys.argv[1]))
