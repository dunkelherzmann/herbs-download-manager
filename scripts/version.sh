#!/bin/sh
# Prints the app version: the major.minor in VERSION, then the number of commits on main since VERSION last
# changed, so every merge to main raises it (0.1.1, 0.1.2, …) and editing VERSION to 0.2 starts at 0.2.0.
# A build with commits main hasn't got (a branch, or local work) keeps main's number and adds the branch and how
# many commits it is ahead: 0.1.157+windows-parity-drop.10. Counting a branch's commits one by one gave it a higher
# number than the squashed merge that followed (0.1.167 on a branch, 0.1.162 on main; bugs.md, 2026-11-17).
# `version.sh --number` prints only the numbers (0.1.157), for fields that take nothing else.
# Without git history (a source download, a shallow clone) the patch number is 0.
set -eu
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BASE="$(tr -d ' \n\r' < "$ROOT/VERSION")"
g() { git -C "$ROOT" "$@" 2>/dev/null; }
PATCH=0
EXTRA=0
since="$(g log -1 --format=%H -- VERSION || true)"
if [ -n "$since" ]; then
    main=""
    for ref in origin/main main; do
        if g rev-parse --verify -q "$ref^{commit}" >/dev/null; then main="$ref"; break; fi
    done
    point=HEAD
    if [ -n "$main" ]; then point="$(g merge-base HEAD "$main" || echo HEAD)"; fi
    if g merge-base --is-ancestor "$since" "$point"; then
        PATCH="$(g rev-list --count "$since..$point")"
        EXTRA="$(g rev-list --count "$point..HEAD")"
    else
        # VERSION changed on this branch: its new number starts here.
        EXTRA="$(g rev-list --count "$since..HEAD")"
    fi
fi
if [ "${1:-}" = "--number" ] || [ "$EXTRA" = 0 ]; then
    echo "$BASE.$PATCH"
else
    branch="$(g rev-parse --abbrev-ref HEAD || echo dev)"
    branch="$(printf '%s' "$branch" | tr -c 'A-Za-z0-9-' '-' | sed 's/^-*//; s/-*$//')"
    [ -n "$branch" ] && [ "$branch" != HEAD ] || branch=dev
    echo "$BASE.$PATCH+$branch.$EXTRA"
fi
