#!/usr/bin/env python3
"""Merge a freshly-built single-item Sparkle appcast into the existing appcast.

Sparkle channel gating (RFC MAC-0006 MAC-T23) needs the latest *stable* and the latest
*beta* build to coexist in one appcast: an item with no ``<sparkle:channel>`` is offered
to everyone, while an item on the ``beta`` channel is offered only to updaters that opted
in. Publishing a single-item appcast that overwrites the previous one would drop the other
channel's build. This script keeps at most one item per channel — the new item replaces any
existing item on the *same* channel, and items on *other* channels are preserved.

Usage:
    merge-appcast.py NEW.xml OUT.xml                 # no prior appcast
    merge-appcast.py NEW.xml EXISTING.xml OUT.xml    # merge into EXISTING

Fail-safe: if EXISTING is missing or unparseable, OUT is just NEW — we never publish a
broken appcast.
"""
import sys
import xml.etree.ElementTree as ET

SPARKLE = "http://www.andymatuschak.org/xml-namespaces/sparkle"
ET.register_namespace("sparkle", SPARKLE)


def channel_of(item: ET.Element) -> str:
    """The item's channel ('' == the default/stable channel offered to everyone)."""
    el = item.find(f"{{{SPARKLE}}}channel")
    return (el.text or "").strip() if el is not None else ""


def main() -> None:
    args = sys.argv[1:]
    if len(args) == 2:
        new_path, existing_path, out_path = args[0], None, args[1]
    elif len(args) == 3:
        new_path, existing_path, out_path = args
    else:
        sys.exit("usage: merge-appcast.py NEW.xml [EXISTING.xml] OUT.xml")

    new_tree = ET.parse(new_path)
    new_channel = new_tree.getroot().find("channel")
    if new_channel is None or not new_channel.findall("item"):
        sys.exit("new appcast has no <channel>/<item>")
    new_ch = channel_of(new_channel.findall("item")[0])

    old_items: list[ET.Element] = []
    if existing_path:
        try:
            old_channel = ET.parse(existing_path).getroot().find("channel")
            if old_channel is not None:
                old_items = old_channel.findall("item")
        except (ET.ParseError, FileNotFoundError, OSError):
            old_items = []  # fail-safe: ignore an unreadable existing appcast

    # Keep existing items only for OTHER channels; the new item owns its own channel.
    kept = [it for it in old_items if channel_of(it) != new_ch]
    for it in kept:
        new_channel.append(it)

    ET.indent(new_tree, space="  ")
    new_tree.write(out_path, xml_declaration=True, encoding="utf-8")
    kept_labels = [channel_of(it) or "stable" for it in kept]
    print(f"merged: new item on '{new_ch or 'stable'}' channel; kept {kept_labels}")


if __name__ == "__main__":
    main()
