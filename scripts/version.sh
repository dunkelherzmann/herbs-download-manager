#!/bin/sh
# Prints the app version: the major.minor in VERSION, then the number of commits since VERSION last
# changed, so every push to main raises it (0.1.1, 0.1.2, …) and editing VERSION to 0.2 starts at 0.2.0.
# Without git history (a source download, a shallow clone) the patch number is 0.
set -eu
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BASE="$(tr -d ' \n\r' < "$ROOT/VERSION")"
PATCH=0
if since="$(git -C "$ROOT" log -1 --format=%H -- VERSION 2>/dev/null)" && [ -n "$since" ]; then
    PATCH="$(git -C "$ROOT" rev-list --count "$since..HEAD")"
fi
echo "$BASE.$PATCH"
