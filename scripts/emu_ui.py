#!/usr/bin/env python3
"""Minimal adb/uiautomator driver for emulator smoke checks (no device evidence).

usage: emu_ui.py tap <text> | wait <text> [timeout_s] | has <text> | dump
Matches visible text or content-description exactly or as a prefix.
"""
import re, subprocess, sys, time, xml.etree.ElementTree as ET

def dump():
    subprocess.run(["adb", "shell", "uiautomator", "dump", "/sdcard/ui.xml"], capture_output=True)
    xml = subprocess.run(["adb", "exec-out", "cat", "/sdcard/ui.xml"], capture_output=True, text=True).stdout
    return ET.fromstring(xml) if xml.strip().startswith("<") else None

def find(text):
    root = dump()
    if root is None:
        return None
    for n in root.iter("node"):
        for attr in ("text", "content-desc"):
            v = n.get(attr, "")
            if v == text or (v.startswith(text) and len(text) > 3):
                m = re.findall(r"\d+", n.get("bounds", ""))
                if len(m) == 4:
                    x1, y1, x2, y2 = map(int, m)
                    return (x1 + x2) // 2, (y1 + y2) // 2
    return None

def main():
    cmd = sys.argv[1]
    if cmd == "dump":
        root = dump()
        for n in root.iter("node"):
            t = n.get("text") or n.get("content-desc")
            if t:
                print(repr(t))
        return 0
    text = sys.argv[2]
    if cmd == "has":
        return 0 if find(text) else 1
    if cmd == "wait":
        deadline = time.time() + float(sys.argv[3] if len(sys.argv) > 3 else 15)
        while time.time() < deadline:
            if find(text):
                return 0
            time.sleep(0.5)
        print(f"timeout waiting for {text!r}", file=sys.stderr)
        return 1
    if cmd == "tap":
        p = find(text)
        if not p:
            print(f"not found: {text!r}", file=sys.stderr)
            return 1
        subprocess.run(["adb", "shell", "input", "tap", str(p[0]), str(p[1])])
        return 0
    return 2

sys.exit(main())
