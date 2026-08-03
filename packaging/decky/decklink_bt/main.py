#!/usr/bin/env python3
"""Decky Loader plugin — launches DeckLink BT Flatpak or local binary."""

import os
import subprocess
import decky_plugin


class Plugin:
    async def launch(self):
        candidates = [
            ["flatpak", "run", "io.github.bastianjosekottekudy_cmyk.DeckLinkBT"],
            [os.path.expanduser("~/.local/bin/decklink-bt")],
            ["/usr/local/bin/decklink-bt"],
        ]
        for cmd in candidates:
            try:
                subprocess.Popen(
                    cmd,
                    stdout=subprocess.DEVNULL,
                    stderr=subprocess.DEVNULL,
                    start_new_session=True,
                )
                return {"ok": True, "cmd": " ".join(cmd)}
            except FileNotFoundError:
                continue
            except Exception as e:
                return {"ok": False, "error": str(e)}
        return {
            "ok": False,
            "error": "DeckLink BT not installed. Run scripts/install-deck.sh in Desktop Mode.",
        }

    async def _main(self):
        decky_plugin.logger.info("DeckLink BT plugin loaded")

    async def _unload(self):
        pass
