"""Regenerate embedded Lucide bodies from the pinned npm tarball. No full icon set is shipped."""
from pathlib import Path
import io
import tarfile
import urllib.request
import xml.etree.ElementTree as ET

VERSION = "1.61.0"
ICONS = {'MEETINGS': 'notebook-pen',
 'ANNOTATIONS': 'pen-tool',
 'GRIP_VERTICAL': 'grip-vertical',
 'TASKS': 'list-checks',
 'MEMORY': 'library',
 'DIARY': 'calendar',
 'SETTINGS': 'settings',
 'DEVICE': 'laptop',
 'PLUS': 'plus',
 'STOP': 'square',
 'CHECK': 'check',
 'CHEVRON_DOWN': 'chevron-down',
 'CHEVRON_RIGHT': 'chevron-right',
 'CHEVRON_LEFT': 'chevron-left',
 'CHEVRONS_UP_DOWN': 'chevrons-up-down',
 'ARROW_RIGHT': 'arrow-right',
 'ARROW_UP': 'arrow-up',
 'X': 'x',
 'MORE': 'ellipsis',
 'WARNING': 'triangle-alert',
 'ERROR': 'circle-alert',
 'HELP': 'circle-question-mark',
 'INFO': 'info',
 'SEARCH': 'search',
 'PANEL_CLOSE': 'panel-left-close',
 'PANEL_OPEN': 'panel-left-open',
 'PROMPTS': 'braces',
 'MIC': 'mic',
 'CIRCLE': 'circle',
 'WAITING': 'circle-pause',
 'BOT': 'bot',
 'MESSAGE_SQUARE': 'message-square',
 'PALETTE': 'palette',
 'CHEVRON_UP': 'chevron-up',
 'COPY': 'copy',
 'LOCK': 'lock',
 'EYE': 'eye',
 'EYE_OFF': 'eye-off',
 'CIRCLE_X': 'circle-x',
 'DASH': 'minus',
 'DONE': 'circle-check',
 'SUMMARY': 'sparkles',
 'UPDATES': 'download',
 'CONNECTED': 'plug',
 'PEOPLE': 'users',
 'FLAG': 'flag',
 'SPELLING': 'spell-check',
 'PHONE_CALL': 'phone-call',
 'LANGUAGES': 'languages',
 'BRAIN': 'brain',
 'AUDIO_LINES': 'audio-lines',
 'TRASH': 'trash-2',
 'VOLUME_X': 'volume-x',
 'FOLDER': 'folder',
 'USER': 'user',
 'FILE_TEXT': 'file-text',
 'KEYBOARD': 'keyboard',
 'ERASER': 'eraser',
 'CLIPBOARD_PASTE': 'clipboard-paste',
 'TIMER': 'timer',
 'CLIPBOARD': 'clipboard',
 'SPEECH': 'speech',
 'POWER': 'power',
 'VOLUME_2': 'volume-2',
 'FLASK': 'flask-conical',
 'CAPTIONS': 'captions',
 'CPU': 'cpu',
 'KEY_ROUND': 'key-round',
 'PACKAGE': 'package',
 'FILE_CLOCK': 'file-clock',
 'GLOBE': 'globe',
 'AUDIO_WAVEFORM': 'audio-waveform',
 'CALENDAR_CHECK': 'calendar-check',
 'LIST_TODO': 'list-todo',
 'FOLDERS': 'folders',
 'RADIO_TOWER': 'radio-tower',
 'MONITOR': 'monitor',
 'SMARTPHONE': 'smartphone',
 'TERMINAL': 'terminal',
 'CODE_XML': 'code-xml',
 'SQUARE_TERMINAL': 'square-terminal',
 'APP_WINDOW': 'app-window',
 'MOUSE_POINTER': 'mouse-pointer-2',
 'NETWORK': 'network',
 'SHIELD_CHECK': 'shield-check',
 'CIRCLE_STOP': 'circle-stop',
 'HOURGLASS': 'hourglass'}
ROOT = Path(__file__).resolve().parents[2]

def main():
    archive = tarfile.open(fileobj=io.BytesIO(urllib.request.urlopen(f"https://registry.npmjs.org/lucide-static/-/lucide-static-{VERSION}.tgz", timeout=30).read()))
    path = ROOT / "src/ui/icons.rs"
    current = path.read_text(encoding="utf-8")
    header = current[:current.index("// Embedded entries")]
    tail = current[current.index("/// Existing controls"):]
    entries = []
    for entry, name in ICONS.items():
        root = ET.fromstring(archive.extractfile(f"package/icons/{name}.svg").read())
        body = ""
        for element in root:
            tag = element.tag.split("}")[-1]
            attributes = dict(element.attrib)
            if tag in ("polyline", "polygon"):
                points = attributes.pop("points")
                attributes["d"] = "M" + points + ("Z" if tag == "polygon" else "")
                tag = "path"
            if tag not in {"path", "rect", "circle", "ellipse", "line"} or "transform" in attributes:
                raise ValueError(f"Rune cannot draw {name}: {tag}")
            attributes.update({"fill": "none", "stroke": "currentColor", "stroke-width": "STROKE_WIDTH", "stroke-linecap": "round", "stroke-linejoin": "round"})
            body += "<" + tag + " " + " ".join(key + '=\"' + value + '\"' for key, value in attributes.items()) + "/>"
        entries.append(f'lucide!({entry}, "lucide-{name}", r#"{body}"#);')
    path.write_text(header + "// Embedded entries\n" + "\n".join(entries) + "\n\n" + tail, encoding="utf-8")
    (ROOT / "assets/licenses/lucide-LICENSE.txt").write_bytes(archive.extractfile("package/LICENSE").read())

if __name__ == "__main__":
    main()
