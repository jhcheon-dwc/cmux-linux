#!/usr/bin/env python3
"""Merge the cmux Claude Code hooks (contrib/claude/hooks.json) into a Claude Code
settings.json (normally ~/.claude/settings.json).

Usage: python3 -I merge_hooks.py HOOKS_JSON SETTINGS_JSON

- Idempotent: a hook command that is already registered for an event is not added again.
- Existing settings and hooks are kept as they are.
- Before writing, the current file is copied to SETTINGS_JSON.bak-<timestamp>.
- If nothing needs to change, the file is not touched (and no backup is made).
"""
import json
import os
import shutil
import sys
import tempfile
import time


def commands_of(entries):
    """All hook commands registered in a list of matcher entries."""
    out = set()
    for entry in entries if isinstance(entries, list) else []:
        for hook in entry.get("hooks", []) if isinstance(entry, dict) else []:
            if isinstance(hook, dict) and "command" in hook:
                out.add(hook["command"])
    return out


def merge(settings, snippet):
    """Merge snippet["hooks"] into settings in place. Returns the number of hooks added."""
    hooks = settings.setdefault("hooks", {})
    if not isinstance(hooks, dict):
        raise ValueError('"hooks" in settings is not an object')
    added = 0
    for event, entries in snippet["hooks"].items():
        existing = hooks.setdefault(event, [])
        if not isinstance(existing, list):
            raise ValueError(f'"hooks.{event}" in settings is not a list')
        have = commands_of(existing)
        for entry in entries:
            new_hooks = [h for h in entry["hooks"] if h["command"] not in have]
            if not new_hooks:
                continue
            new_entry = dict(entry)
            new_entry["hooks"] = new_hooks
            existing.append(new_entry)
            have.update(h["command"] for h in new_hooks)
            added += len(new_hooks)
    return added


def main(argv):
    if len(argv) != 3:
        print(__doc__.strip(), file=sys.stderr)
        return 2
    # Resolve symlinks (dotfile managers) so the link's target is updated, not replaced.
    hooks_path, settings_path = argv[1], os.path.realpath(argv[2])
    with open(hooks_path, encoding="utf-8") as f:
        snippet = json.load(f)

    if os.path.exists(settings_path):
        with open(settings_path, encoding="utf-8") as f:
            text = f.read()
        try:
            settings = json.loads(text) if text.strip() else {}
        except json.JSONDecodeError as e:
            print(f"error: {settings_path} is not valid JSON ({e}); not modified", file=sys.stderr)
            return 1
        if not isinstance(settings, dict):
            print(f"error: {settings_path} is not a JSON object; not modified", file=sys.stderr)
            return 1
    else:
        settings = {}

    try:
        added = merge(settings, snippet)
    except ValueError as e:
        print(f"error: {e}; {settings_path} not modified", file=sys.stderr)
        return 1
    if added == 0:
        print(f"cmux hooks already present in {settings_path}")
        return 0

    directory = os.path.dirname(os.path.abspath(settings_path))
    os.makedirs(directory, exist_ok=True)
    if os.path.exists(settings_path):
        backup = f"{settings_path}.bak-{time.strftime('%Y%m%d%H%M%S')}"
        shutil.copy2(settings_path, backup)
        print(f"backup: {backup}")
    fd, tmp = tempfile.mkstemp(dir=directory, prefix=".settings.", suffix=".json")
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            json.dump(settings, f, indent=2, ensure_ascii=False)
            f.write("\n")
        if os.path.exists(settings_path):
            shutil.copymode(settings_path, tmp)
        os.replace(tmp, settings_path)
    except BaseException:
        if os.path.exists(tmp):
            os.unlink(tmp)
        raise
    print(f"added {added} cmux hook(s) to {settings_path}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
