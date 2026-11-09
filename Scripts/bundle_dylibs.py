#!/usr/bin/env python3
"""Copies every non-system dylib an executable needs into the app's Frameworks folder.

Usage: bundle_dylibs.py <executable> <frameworks-dir>

Rewrites install names and references to @rpath/<name>, gives the executable an
@executable_path/../Frameworks rpath, and drops absolute rpaths (for example into Homebrew).
Prints one line per bundled library: "<name>\t<original real path>".
"""

import os
import re
import shutil
import subprocess
import sys

SYSTEM_PREFIXES = ("/usr/lib/", "/System/")


def run(*args):
    return subprocess.run(args, check=True, capture_output=True, text=True).stdout


def dependencies(path):
    """Install names this Mach-O links against (excluding its own id)."""
    lines = run("otool", "-L", path).splitlines()[1:]
    names = [line.strip().split(" (compatibility")[0] for line in lines if line.strip()]
    own_id = run("otool", "-D", path).splitlines()[1:]
    return [n for n in names if n not in own_id]


def rpaths(path):
    output = run("otool", "-l", path)
    return re.findall(r"cmd LC_RPATH\n\s+cmdsize \d+\n\s+path (.+?) \(offset", output)


def resolve(name, loader, executable):
    """Resolves an install name the way dyld would for `loader`."""
    loader_dir = os.path.dirname(os.path.realpath(loader))
    if name.startswith("@loader_path/"):
        return os.path.realpath(os.path.join(loader_dir, name[len("@loader_path/"):]))
    if name.startswith("@executable_path/"):
        return os.path.realpath(os.path.join(os.path.dirname(executable), name[len("@executable_path/"):]))
    if name.startswith("@rpath/"):
        for rpath in rpaths(loader):
            base = rpath.replace("@loader_path", loader_dir).replace("@executable_path", os.path.dirname(executable))
            candidate = os.path.join(base, name[len("@rpath/"):])
            if os.path.exists(candidate):
                return os.path.realpath(candidate)
        return None
    return os.path.realpath(name) if os.path.exists(name) else None


def needs_bundling(name):
    return not name.startswith(SYSTEM_PREFIXES)


def main():
    executable, frameworks = sys.argv[1], sys.argv[2]
    os.makedirs(frameworks, exist_ok=True)

    bundled = {}  # bundled file name -> original real path
    # (Mach-O to edit, where it came from). Relative install names resolve against the origin.
    queue = [(executable, executable)]
    changes = {}  # Mach-O path -> [(old install name, new install name)]
    while queue:
        current, origin = queue.pop()
        for name in dependencies(current):
            if not needs_bundling(name):
                continue
            original = resolve(name, origin, executable)
            if original is None:
                # Swift runtime stubs resolved through the OS (e.g. @rpath/libswift_*.dylib).
                if name.startswith("@rpath/libswift"):
                    continue
                sys.exit(f"error: cannot resolve {name} needed by {current}")
            file_name = os.path.basename(name)
            changes.setdefault(current, []).append((name, f"@rpath/{file_name}"))
            if file_name not in bundled:
                bundled[file_name] = original
                destination = os.path.join(frameworks, file_name)
                shutil.copy2(original, destination)
                os.chmod(destination, 0o755)
                queue.append((destination, original))

    for path, edits in changes.items():
        args = []
        for old, new in edits:
            if old != new:
                args += ["-change", old, new]
        if args:
            run("install_name_tool", *args, path)

    for file_name in bundled:
        path = os.path.join(frameworks, file_name)
        run("install_name_tool", "-id", f"@rpath/{file_name}", path)
        for rpath in rpaths(path):
            run("install_name_tool", "-delete_rpath", rpath, path)
        run("install_name_tool", "-add_rpath", "@loader_path", path)

    for rpath in rpaths(executable):
        if rpath.startswith("/") and not rpath.startswith(SYSTEM_PREFIXES):
            run("install_name_tool", "-delete_rpath", rpath, executable)
    if "@executable_path/../Frameworks" not in rpaths(executable):
        run("install_name_tool", "-add_rpath", "@executable_path/../Frameworks", executable)

    for file_name, original in sorted(bundled.items()):
        print(f"{file_name}\t{original}")


if __name__ == "__main__":
    main()
