#!/usr/bin/env python3
"""Writes the app's third-party notices and license texts.

Usage: write-notices.py <FFmpeg install prefix> <resources dir> <where FFmpeg is bundled>

Covers every crate linked into the app on this platform. Fails if one is available only under
a GPL-family license, since the app must stay distributable with LGPL FFmpeg.
"""

import glob
import json
import os
import re
import shutil
import subprocess
import sys

prefix, resources, bundled_where = sys.argv[1:4]
root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
info = dict(line.rstrip("\n").split("=", 1) for line in open(os.path.join(prefix, "BUILD-INFO.txt")) if "=" in line)

licenses = os.path.join(resources, "Licenses", "FFmpeg")
os.makedirs(licenses, exist_ok=True)
for name in ("COPYING.LGPLv2.1", "LICENSE.md"):
    shutil.copy(os.path.join(prefix, "share", "licenses", name), os.path.join(licenses, name))


def host():
    output = subprocess.run(["rustc", "-vV"], check=True, capture_output=True, text=True).stdout
    return re.search(r"^host: (.+)$", output, re.M).group(1)


def linked_packages():
    metadata = json.loads(subprocess.run(
        ["cargo", "metadata", "--locked", "--format-version", "1", "--filter-platform", host(),
         "--manifest-path", os.path.join(root, "Cargo.toml")],
        check=True, capture_output=True, text=True).stdout)
    packages = {package["id"]: package for package in metadata["packages"]}
    nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
    start = next(identifier for identifier, package in packages.items() if package["name"] == "skia-ui")
    seen, queue = set(), [start]
    while queue:
        current = queue.pop()
        if current in seen:
            continue
        seen.add(current)
        for dep in nodes[current]["deps"]:
            if any(kind["kind"] is None for kind in dep["dep_kinds"]):
                queue.append(dep["pkg"])
    ours = {"skia-ui"}
    return sorted(
        (packages[identifier] for identifier in seen
         if packages[identifier]["name"] not in ours
         and not packages[identifier]["name"].startswith("loudible-")),
        key=lambda package: (package["name"], package["version"]))


def is_copyleft_only(expression):
    if not expression:
        return False
    alternatives = re.split(r"\s+OR\s+|/", expression.replace("(", " ").replace(")", " "))
    return all("GPL" in alternative for alternative in alternatives)


packages = linked_packages()
rejected = [f"{p['name']} {p['version']}: {p['license']}" for p in packages if is_copyleft_only(p["license"])]
if rejected:
    sys.exit("error: copyleft-only dependencies:\n  " + "\n  ".join(rejected))

texts = []
listing = []
for package in packages:
    directory = os.path.dirname(package["manifest_path"])
    files = sorted(
        path for pattern in ("LICEN[CS]E*", "COPYING*", "NOTICE*", "UNLICENSE*")
        for path in glob.glob(os.path.join(directory, pattern)) if os.path.isfile(path))
    if package.get("license_file"):
        named = os.path.join(directory, package["license_file"])
        if os.path.isfile(named) and named not in files:
            files.append(named)
    expression = package["license"] or "see license file"
    listing.append(f"  {package['name']} {package['version']}: {expression}")
    if files:
        texts.append(f"{'=' * 72}\n{package['name']} {package['version']} ({expression})\n{'=' * 72}")
        for path in files:
            with open(path, errors="replace") as file:
                texts.append(f"--- {os.path.basename(path)}\n{file.read().strip()}\n")

os.makedirs(os.path.join(resources, "Licenses"), exist_ok=True)
with open(os.path.join(resources, "Licenses", "rust-crates.txt"), "w") as file:
    file.write("\n".join(texts))

crates = "\n".join(listing)
notices = f"""Loudible third-party notices

FFmpeg {info['version']}
This software uses libraries from the FFmpeg project (https://ffmpeg.org) under the
GNU Lesser General Public License version 2.1 or later: libavformat, libavcodec, and
libavutil, {bundled_where} as separate dynamic libraries, unmodified.
FFmpeg is a trademark of Fabrice Bellard, originator of the FFmpeg project.

License text: Licenses/FFmpeg/COPYING.LGPLv2.1 (see also Licenses/FFmpeg/LICENSE.md).

Source code: the exact source used is the official release tarball
  {info['url']}
  SHA-256 {info['sha256']}
It is also published alongside each Loudible release, together with the build
script (Scripts/build-ffmpeg.sh). FFmpeg was configured with:
  {info['configure']}

You may replace the bundled FFmpeg libraries with your own builds of the same version.

Skia
This software embeds Skia (https://skia.org), BSD 3-Clause License, with the libraries
its prebuilt binaries include. See https://skia.googlesource.com/skia/+/refs/heads/main/LICENSE.

Rust crates
License texts: Licenses/rust-crates.txt.
{crates}
"""
with open(os.path.join(resources, "THIRD_PARTY_NOTICES.txt"), "w") as file:
    file.write(notices)
