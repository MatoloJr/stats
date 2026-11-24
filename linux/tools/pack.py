#!/usr/bin/env python3
"""Build the release zip with paths kept as given (python -m zipfile -c
flattens files to their basename). Usage: pack.py OUT.zip FILE_OR_DIR..."""
import os
import sys
import zipfile

out, *sources = sys.argv[1:]
with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as z:
    for src in sources:
        paths = [src] if os.path.isfile(src) else sorted(
            os.path.join(d, f) for d, _, files in os.walk(src) for f in files)
        for path in paths:
            z.write(path, path.replace(os.sep, "/"))
