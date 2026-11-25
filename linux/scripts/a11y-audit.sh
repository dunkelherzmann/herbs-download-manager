#!/bin/sh
# Every control explains itself (§3, §4): opens each window of the installed Flatpak in a virtual KWin session (never
# the desktop), reads what screen readers see through AT-SPI, and fails if a button, field, checkbox or dropdown has
# no name or no hover text (its description). Run on the Linux machine after `linux/scripts/build.sh --install`.
# Needs kwin_wayland and the app-parity skill's atspi-dump.py.
set -u
DUMP="$HOME/.agents/library/app-parity/scripts/atspi-dump.py"
WORK="$HOME/.cache/hdm-a11y"
mkdir -p "$WORK"
export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
cat > "$WORK/one.sh" <<SCRIPT
#!/bin/sh
D=$WORK/data-\$1; rm -rf "\$D"; mkdir -p "\$D"
flatpak run --instance-id-fd=3 --env=HD_DATA_DIR=\$D --env=HD_DOWNLOADS_DIR=\$D org.herbsdownloadmanager.app --add http://127.0.0.1:9/a.bin \
    --show \$1 http://127.0.0.1:9/b.bin http://127.0.0.1:9/c.bin 3>"\$D/instance" &
sleep 6
# By the test copy's own process: by name it could read the user's copy, which may be running too.
# flatpak ps's child-pid is the sandbox's inner helper; the app is its child.
HELPER=\$(flatpak ps --columns=instance,child-pid | awk -v i="\$(cat "\$D/instance")" '\$1 == i {print \$2}')
PID=\$(pgrep -P "\$HELPER" | head -1)
python3 "$DUMP" "\${PID:-herbs-download-manager}" "\$2" --wait 15 >/dev/null 2>&1
# Only this test instance: killing by the app id also ended the user's own copy, which keeps running in the
# background since #44 (2026-11-25).
flatpak kill "\$(cat "\$D/instance")" 2>/dev/null
SCRIPT
chmod +x "$WORK/one.sh"
for w in main download-window settings new-download add-links batch; do
    rm -f "$WORK/$w.json"
    timeout 60 kwin_wayland --virtual --no-lockscreen --width 1300 --height 800 --socket hdm-a11y \
        --exit-with-session "$WORK/one.sh $w $WORK/$w.json" >"$WORK/$w.log" 2>&1
done
python3 - "$WORK" <<'PY'
import json, os, sys
work = sys.argv[1]
roles = {"button", "push button", "toggle button", "check box", "text", "entry", "spin button", "combo box", "switch", "menu button", "radio button"}
failed = False
for w in ["main", "download-window", "settings", "new-download", "add-links", "batch"]:
    path = os.path.join(work, w + ".json")
    if not os.path.exists(path):
        print(f"{w}: the window didn't open (see {work}/{w}.log)"); failed = True; continue
    data = json.load(open(path))
    items = data if isinstance(data, list) else data.get("elements") or data.get("nodes") or []
    controls = [e for e in items if e.get("role") in roles]
    bad = sorted({(e.get("role"), e.get("label") or "", e.get("help") or "") for e in controls
                  if not (e.get("label") or "").strip() or not (e.get("help") or "").strip()})
    print(f"{w}: {len(controls)} controls, {len(bad)} without a name or hover text")
    for role, label, help_ in bad:
        print(f"    {role} name={label!r} hover={help_!r}"); failed = True
sys.exit(1 if failed else 0)
PY
