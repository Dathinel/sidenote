"""Check the real offscreen app trees for Ask controls and the requested render matrix."""
import json
from pathlib import Path
import struct
import sys

root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("artifacts/ask-models")


def shot(scene, width, theme):
    name = f"{scene}-{width}-{theme}"
    tree = json.loads((root / f"{name}.json").read_text(encoding="utf-8-sig"))
    assert not tree["warnings"], (name, tree["warnings"])
    png = (root / f"{name}.png").read_bytes()
    assert struct.unpack(">II", png[16:24]) == (width, 800), name
    return tree["elements"]


def keyed(elements, key):
    return next(node for node in elements if node.get("key") == key)


for width in (1280, 640):
    for theme in ("light", "dark"):
        page = shot("models-and-agents", width, theme)
        assert keyed(page, "settings-title")["text"] == "Models and agents"
        assert not any("AI helpers" in node.get("text", "") for node in page)
        for key in ("ask-agent-row", "ask-model-row", "ask-meeting-thinking-row", "ask-chat-thinking-row"):
            keyed(page, key)
        menu = shot("models-and-agents-menu", width, theme)
        assert {"Sonnet 5.5", "Opus 5.5", "Haiku 5.5", "Other\u2026"} <= {
            node.get("label") for node in menu if node.get("role") == "option"
        }
        assert not any(node.get("key") == "ask-settings-model-input" for node in menu)
        other = shot("models-and-agents-other", width, theme)
        keyed(other, "ask-settings-model-input")
        keyed(other, "ask-settings-model-save")
        for elements in (page, menu, other):
            assert not any("helper" in str(node.get(field, "")).lower()
                           for node in elements for field in ("text", "label", "help"))
        for scene, level in (("ask-thinking-picker", "Medium"), ("ask-in-meeting", "Low")):
            elements = shot(scene, width, theme)
            trigger = keyed(elements, "ask-thinking")
            assert trigger.get("label") == f"Thinking: {level}", (scene, trigger)
            x, _, w, h = trigger["rect"]
            assert x >= 0 and x + w <= width + 0.1 and w > 0 and h > 0
            assert {"Low", "Medium", "High"} <= {node.get("text") for node in elements}
        streamed = shot("ask-streaming", width, theme)
        keyed(streamed, "ask-streaming-answer")
        assert not any(node.get("key") == "ask-progress" for node in streamed)
        opencode = shot("ask-opencode", width, theme)
        assert not any(node.get("key") == "ask-thinking" for node in opencode)
        assert "no separate effort flag" in keyed(opencode, "ask-model").get("help", "")
print("PASS: 28 offscreen PNGs and trees; offered-model menu and Other field, agent wording, Low/Medium defaults, all three thinking options, streamed text replacing the timer, and the OpenCode explanation.")
