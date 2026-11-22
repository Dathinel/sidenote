"""Find the GNU toolchain bin folder for maintainer tools."""

import os
from pathlib import Path
import shutil


def gnu_bin():
    folder = os.environ.get("SIDENOTE_GNU_BIN")
    if folder is not None:
        if folder and Path(folder).is_dir():
            return Path(folder).resolve()
        raise SystemExit("SIDENOTE_GNU_BIN must point to an existing GNU toolchain bin folder.")
    gcc = shutil.which("gcc")
    if gcc:
        return Path(gcc).absolute().parent
    raise SystemExit("Set SIDENOTE_GNU_BIN to a GNU toolchain bin folder or put gcc on PATH.")
