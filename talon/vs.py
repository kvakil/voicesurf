from pathlib import Path
from typing import List
from talon import cron, Module, Context, fs
import json

# TODO(kvakil): better way to get XDG_RUNTIME_DIR to Talon?
VOICESURF_PATH = Path.home() / ".run" / "voicesurf"

mod = Module()
mod.list("hints", desc="hints from the web page")

ctx = Context()
ctx.lists["self.hints"] = {}

current_tab_id = None


@mod.action_class
class Actions:
    def surf(hints: List[str]):
        """Surf to hint"""
        with (VOICESURF_PATH / "output" / "v0").open("w") as fp:
            # TODO(kvakil): use a temporary file to make this atomic?
            json.dump(
                {"Query": {"query": " ".join(hints), "tabId": current_tab_id}}, fp
            )


def update_surf(_, _2):
    global current_tab_id
    with (VOICESURF_PATH / "input" / "v0").open() as fp:
        message = json.loads(fp)
        hints = message["UpdateTalonRequest"]["words"]
        current_tab_id = message["UpdateTalonRequest"]["tabId"]

    ctx.lists["self.hints"] = {hint_text: str(hint_text) for hint_text in hints}


@mod.capture(rule="{self.hints}+")
def hints(m) -> List[str]:
    return m.hints_list


fs.watch(str(VOICESURF_PATH / "input"), update_surf)
