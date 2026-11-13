#!/usr/bin/env bash
# KetraCopy FEATURE-INTEROPERABILITY test suite (Linux).
#
# While test_integration.sh exercises features one at a time, this suite
# verifies that features that are supposed to WORK TOGETHER actually do:
#
#   settings file ↔ engine defaults        (verify / limit / clone propagation)
#   throttle × resume × verify × manifest  (one interrupted, limited, verified run)
#   conflicts × manifest refresh           (rename/skip keep the manifest true)
#   sparse × manifest, filters × flatten,  dry-run ↔ real-run parity
#   move × verify                          (same-fs rename + cross-fs fallback)
#   clipboard ↔ engine                     (real X11 round-trip under Xvfb:
#                                           clip put → selection → clip paste)
#   shell scripts ↔ CLI ↔ clipboard ↔ engine (generated Nautilus script chain)
#   mDNS discovery ↔ receive ↔ send ↔ integrity (LAN loopback, LocalSend v2)
#   GUI smoke under Xvfb, artifact cross-consistency (dist/portable/deb/sums)
#   cross-binary manifest compatibility, bench
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
BIN="$ROOT/target/release/ketra"
GUI="$ROOT/target/release/ketracopy-gui"
WORK="$(mktemp -d /tmp/ketra-ix.XXXXXX)"
REAL_HOME="$HOME"
export HOME="$WORK/home"
export XDG_CONFIG_HOME="$WORK/home/.config"
mkdir -p "$HOME"
PASS=0; FAIL=0
SHA() { sha256sum "$1" | cut -d' ' -f1; }

say()  { printf '\n== %s ==\n' "$*"; }
ok()   { PASS=$((PASS+1)); printf '  PASS %s\n' "$*"; }
bad()  { FAIL=$((FAIL+1)); printf '  FAIL %s\n' "$*"; }
check(){ if [ "$1" = "0" ]; then ok "$2"; else bad "$2"; fi }
checkn(){ if [ "$1" -ne "0" ]; then ok "$2"; else bad "$2"; fi }

[ -x "$BIN" ] || { echo "building…"; (cd "$ROOT" && cargo build --release -p ketra-cli) || exit 1; }

# ---------------------------------------------------------------------------
say "fixture tree (unicode, empty, sparse, 8 MiB payload)"
A="$WORK/treeA"
mkdir -p "$A/sub/deep" "$A/empty" "$A/.config"
python3 - "$A" <<'EOF'
import os, random, sys
random.seed(7); root = sys.argv[1]
def blob(p, n):
    open(p, 'wb').write(bytes(random.getrandbits(8) for _ in range(n)))
open(f'{root}/readme.txt','w').write('interop fixture\n'*40)
blob(f'{root}/bin.dat', 300_000)
blob(f'{root}/sub/data.bin', 1_100_000)
open(f'{root}/sub/deep/notes.md','w').write('# interop\n')
open(f'{root}/0byte.bin','w')
open(f'{root}/репорт タイトル #1.txt','w').write('unicode ok')
open(f'{root}/.config/dot.cfg','w').write('dotfile\n')
open(f'{root}/skipme.tmp','w').write('temp')
with open(f'{root}/sub/sparse.bin','wb') as f:
    f.truncate(32*1024*1024)
    f.seek(8*1024*1024);  f.write(b'A'*4096)
    f.seek(24*1024*1024); f.write(b'B'*4096)
blob(f'{root}/big.bin', 8_388_608)          # 8 MiB — throttle/resume payload
EOF
ok "fixtures created"

# Xvfb for clipboard + GUI sections
XVFB_PID=""
if command -v Xvfb >/dev/null 2>&1; then
  Xvfb :97 -screen 0 1024x768x24 >/dev/null 2>&1 & XVFB_PID=$!
  for i in $(seq 1 30); do [ -e /tmp/.X11-unix/X97 ] && break; sleep 0.1; done
  if [ -e /tmp/.X11-unix/X97 ]; then export DISPLAY=:97; ok "Xvfb :97 up (clipboard/GUI sections live)"; else
    kill $XVFB_PID 2>/dev/null; XVFB_PID=""; bad "Xvfb failed to start"; fi
else
  bad "Xvfb missing — clipboard tests run in degraded mode"
fi

cleanup() {
  [ -n "$XVFB_PID" ] && kill "$XVFB_PID" 2>/dev/null
  pkill -f "ketra __clip-server" 2>/dev/null
  pkill -f "ketra receive" 2>/dev/null
}
trap cleanup EXIT

# ===========================================================================
say "S1. settings file → engine defaults (CLI/GUI shared ketracopy.json)"
CFG="$XDG_CONFIG_HOME/ketracopy/ketracopy.json"
mkdir -p "$(dirname "$CFG")"
printf '{"verify":"manifest","speed_limit_bps":2000000,"use_clone":false}' > "$CFG"
"$BIN" config >"$WORK/cfg.out" 2>&1; check $? "ketra config exits 0"
grep -q "verify=manifest" "$WORK/cfg.out" && ok "config shows verify=manifest" || bad "config shows verify=manifest ($(cat "$WORK/cfg.out" | tail -1))"
grep -q "limit=2000000"  "$WORK/cfg.out" && ok "config shows limit=2000000"  || bad "config shows limit=2000000"

# S1a: settings verify=manifest honored by a plain copy (no --verify flag)
"$BIN" copy "$A/readme.txt" "$A/bin.dat" -d "$WORK/s1a" --progress quiet -y >"$WORK/s1a.log" 2>&1
check $? "copy with settings verify=manifest exits 0"
[ -f "$WORK/s1a/.ketra-manifest.json" ] && ok "manifest written WITHOUT --verify flag (settings honored)" || bad "manifest written WITHOUT --verify flag"
[ -f "$WORK/s1a/readme.txt" ] && ok "files copied (engine ran normally)" || bad "files copied"

# S1b: settings speed limit honored on 8 MiB payload
# (token bucket: ~2s burst allowance then throttled — cf. integration test 6)
t0=$(date +%s%N)
"$BIN" copy "$A/big.bin" -d "$WORK/s1b" --progress quiet -y >/dev/null 2>&1
rc=$?; t1=$(date +%s%N); dt=$(( (t1-t0)/1000000 ))
check $rc "copy under settings limit exits 0"
[ "$dt" -ge 1800 ] && ok "settings speed limit held (took ${dt}ms ≥ 1800ms, burst-adjusted)" || bad "settings speed limit held (took ${dt}ms)"

# S1c: --limit 0 clears the settings limit (same 8 MiB now fast)
t0=$(date +%s%N)
"$BIN" copy "$A/big.bin" -d "$WORK/s1c" --limit 0 --progress quiet -y >/dev/null 2>&1
rc=$?; t1=$(date +%s%N); dt=$(( (t1-t0)/1000000 ))
check $rc "--limit 0 accepted"
[ "$dt" -lt 2500 ] && ok "--limit 0 cleared the settings limit (took ${dt}ms < 2500ms)" || bad "--limit 0 cleared the settings limit (took ${dt}ms)"

# S1d: use_clone=false accepted end-to-end
"$BIN" copy "$A/sub" -d "$WORK/s1d" --progress quiet -y >/dev/null 2>&1
check $? "copy with use_clone=false exits 0"
rm -f "$CFG"

# ===========================================================================
say "S2. verify × manifest × conflicts × filters × flatten"
# S2a: exclude-filtered inline copy
"$BIN" copy "$A" -d "$WORK/s2a" --exclude '*.tmp' --verify inline --progress quiet -y >"$WORK/s2a.log" 2>&1
check $? "copy --exclude --verify inline exits 0"
[ -f "$WORK/s2a/readme.txt" ] && [ -f "$WORK/s2a/sub/data.bin" ] && ok "structure preserved (dir contents at dest root)" || bad "structure preserved"
[ ! -e "$WORK/s2a/skipme.tmp" ] && ok "exclude filter applied" || bad "exclude filter applied"
cmp -s "$A/sub/data.bin" "$WORK/s2a/sub/data.bin" && ok "bit-identical after inline verify" || bad "bit-identical after inline verify"
[ ! -e "$WORK/s2a/empty" ] || [ -d "$WORK/s2a/empty" ]; true # dirs optional here

# S2b: manifest over the copied tree → verify clean
"$BIN" manifest --root "$WORK/s2a" --out "$WORK/m.json" >/dev/null 2>&1; check $? "manifest command exits 0"
"$BIN" verify --root "$WORK/s2a" --manifest "$WORK/m.json" --quiet >/dev/null 2>&1
check $? "manifest verify clean"

# S2c: corruption → verify(4) catches it; `test` stays a read-test (exit 0
# on a readable-but-altered file is correct: it has no reference hash)
VICTIM="$WORK/s2a/sub/data.bin"
python3 - "$VICTIM" <<'EOF'
import sys
p=sys.argv[1]
with open(p,'r+b') as f: f.seek(1000); b=f.read(1); f.seek(1000); f.write(bytes([b[0]^0xFF]))
EOF
"$BIN" verify --root "$WORK/s2a" --manifest "$WORK/m.json" --quiet >/dev/null 2>&1; r1=$?
"$BIN" test "$A/sub/data.bin" --hash sha256 >/dev/null 2>&1; r2=$?
checkn $r1 "verify exits 4 on corruption (got $r1)"
check $r2 "test passes a healthy file (read-test, got $r2)"
# restore
rm -rf "$WORK/s2a"; "$BIN" copy "$A" -d "$WORK/s2a" --exclude '*.tmp' --verify manifest --progress quiet -y >/dev/null 2>&1
"$BIN" verify --root "$WORK/s2a" --quiet >/dev/null 2>&1 && ok "verify clean after recopy (default manifest path)" || bad "verify clean after recopy"

# S2d: conflict=rename keeps the manifest true over the renamed copy
printf 'changed content v2\n' > "$A/readme.txt"
"$BIN" copy "$A" -d "$WORK/s2a" --exclude '*.tmp' --conflict rename --verify manifest --progress quiet -y >/dev/null 2>&1
check $? "conflict=rename copy exits 0"
[ -f "$WORK/s2a/readme (2).txt" ] && ok "renamed to 'readme (2).txt'" || bad "renamed to 'readme (2).txt'"
"$BIN" verify --root "$WORK/s2a" --quiet >/dev/null 2>&1 && ok "manifest refreshed over renamed file (verify clean)" || bad "manifest refreshed over renamed file"

# S2e: conflict=skip leaves dest byte-identical
H_BEFORE=$(SHA "$WORK/s2a/sub/data.bin")
"$BIN" copy "$A" -d "$WORK/s2a" --exclude '*.tmp' --conflict skip --progress quiet -y >/dev/null 2>&1
check $? "conflict=skip run exits 0"
[ "$(SHA "$WORK/s2a/sub/data.bin")" = "$H_BEFORE" ] && ok "existing file untouched by skip" || bad "existing file untouched by skip"

# S2f: flatten × include combo
"$BIN" copy "$A" -d "$WORK/s2f" --flatten --include '*.txt' --include '*.md' --progress quiet -y >/dev/null 2>&1
check $? "flatten+include copy exits 0"
[ -f "$WORK/s2f/readme.txt" ] && [ ! -d "$WORK/s2f/sub" ] && ok "flattened + filtered (no subdir)" || bad "flattened + filtered"

# S2g: unicode / empty dir / 0-byte file through manifest round-trip
"$BIN" copy "$A" -d "$WORK/s2g" --verify manifest --progress quiet -y >/dev/null 2>&1
check $? "unicode+empty+0byte copy exits 0"
[ -f "$WORK/s2g/репорт タイトル #1.txt" ] && ok "unicode filename preserved" || bad "unicode filename preserved"
[ -f "$WORK/s2g/0byte.bin" ] && ok "0-byte file present" || bad "0-byte file present"
"$BIN" verify --root "$WORK/s2g" --quiet >/dev/null 2>&1 && ok "manifest verify incl. unicode + 0-byte" || bad "manifest verify incl. unicode + 0-byte"

# ===========================================================================
say "S3. sparse × manifest (both modes) + all-disable combo + net-streams"
BLK_SRC=$(stat -c %b "$A/sub/sparse.bin")
"$BIN" copy "$A/sub/sparse.bin" -d "$WORK/s3a" --sparse on --verify manifest --progress quiet -y >/dev/null 2>&1
check $? "sparse-on copy exits 0"
BLK_DST=$(stat -c %b "$WORK/s3a/sparse.bin")
[ "$BLK_DST" -le "$BLK_SRC" ] && ok "sparse preserved (dst blocks $BLK_DST ≤ src $BLK_SRC)" || bad "sparse preserved (dst $BLK_DST vs src $BLK_SRC)"
"$BIN" verify --root "$WORK/s3a" --quiet >/dev/null 2>&1 && ok "manifest verify clean over sparse copy" || bad "manifest verify clean over sparse copy"
"$BIN" copy "$A/sub/sparse.bin" -d "$WORK/s3b" --sparse off --progress quiet -y >/dev/null 2>&1
check $? "sparse-off copy exits 0"
BLK_OFF=$(stat -c %b "$WORK/s3b/sparse.bin")
[ "$BLK_OFF" -ge "$BLK_DST" ] && ok "sparse-off allocated ≥ sparse-on ($BLK_OFF ≥ $BLK_DST)" || bad "sparse-off block accounting"
"$BIN" copy "$A" -d "$WORK/s3c" --no-clone --sparse off --no-times --no-attrs --verify inline --progress quiet -y >/dev/null 2>&1
check $? "disable-combo (--no-clone --sparse off --no-times --no-attrs) exits 0"
cmp -s "$A/sub/data.bin" "$WORK/s3c/sub/data.bin" && ok "content identical under disable-combo" || bad "content identical under disable-combo"
"$BIN" copy "$A/readme.txt" -d "$WORK/s3d" --net-streams 4 --verify inline --progress quiet -y >/dev/null 2>&1
check $? "--net-streams 4 accepted on local fs"

# ===========================================================================
say "S4. dry-run ↔ real-run parity"
"$BIN" copy "$A" -d "$WORK/s4" --exclude '*.tmp' --dry-run --progress json -y >"$WORK/s4.log" 2>&1
check $? "dry-run exits 0"
[ ! -e "$WORK/s4" ] && ok "dry-run wrote nothing" || bad "dry-run wrote nothing"
N_PLAN=$(grep -oE '"event":"started","total_files":[0-9]+' "$WORK/s4.log" | grep -oE '[0-9]+$' | head -1)
[ -n "$N_PLAN" ] && ok "dry-run JSON plan reports total_files=$N_PLAN" || bad "dry-run JSON plan (see $WORK/s4.log)"
# every planned file must appear with action=planned
N_PLANNED=$(grep -c '"action":"planned"' "$WORK/s4.log")
"$BIN" copy "$A" -d "$WORK/s4" --exclude '*.tmp' --progress quiet -y >/dev/null 2>&1
N_DST=$(cd "$WORK/s4" && find . -type f | wc -l)
[ -n "$N_PLAN" ] && [ "$N_PLAN" = "$N_DST" ] && [ "$N_PLANNED" = "$N_DST" ] \
  && ok "plan count == planned events == real count ($N_PLAN/$N_PLANNED/$N_DST)" \
  || bad "plan count == planned == real ($N_PLAN/$N_PLANNED vs $N_DST)"

# ===========================================================================
say "S5. throttle × interrupt × resume × verify × manifest (one flow)"
D="$WORK/s5"; rm -rf "$D"
"$BIN" copy "$A/big.bin" -d "$D" --limit 1M --progress quiet -y >/dev/null 2>&1 &
CPID=$!
sleep 2.5
kill -INT $CPID 2>/dev/null; kill -KILL $CPID 2>/dev/null; wait $CPID 2>/dev/null
[ -f "$D/.ketracopy-journal.jsonl" ] && ok "journal present after interrupt" || bad "journal present after interrupt"
"$BIN" copy "$A/big.bin" -d "$D" --verify manifest --progress quiet -y >/dev/null 2>&1
check $? "resume run exits 0"
cmp -s "$A/big.bin" "$D/big.bin" && ok "resumed file byte-identical" || bad "resumed file byte-identical"
"$BIN" verify --root "$D" --quiet >/dev/null 2>&1 && ok "manifest verify clean after resume" || bad "manifest verify clean after resume"
"$BIN" copy "$A/readme.txt" -d "$WORK/s5nr" --no-resume --verify inline --progress quiet -y >/dev/null 2>&1
check $? "--no-resume accepted on a clean run"

# ===========================================================================
say "S6. move × verify (same-fs rename and cross-fs fallback)"
"$BIN" move "$A/sub" -d "$WORK/s6a" --verify manifest --progress quiet -y >/dev/null 2>&1
check $? "move exits 0"
# the engine moves the file *contents*; the emptied source dir shell may
# remain — assert the FILES are gone and the dest has them
[ ! -f "$A/sub/data.bin" ] && [ ! -f "$A/sub/deep/notes.md" ] && [ -f "$WORK/s6a/data.bin" ] \
  && ok "source files removed, dest present" || bad "source files removed, dest present"
"$BIN" verify --root "$WORK/s6a" --quiet >/dev/null 2>&1 && ok "manifest verify clean after move" || bad "manifest verify clean after move"
if [ -d /dev/shm ] && [ -w /dev/shm ]; then
  mkdir -p /dev/shm/ketra-ix-move
  "$BIN" move "$A/readme.txt" -d /dev/shm/ketra-ix-move --verify inline --progress quiet -y >/dev/null 2>&1; r=$?
  check $r "cross-fs move exits 0"
  [ ! -f "$A/readme.txt" ] && [ -f /dev/shm/ketra-ix-move/readme.txt ] && ok "cross-fs move: source gone, dest present" || bad "cross-fs move: source gone, dest present"
  rm -rf /dev/shm/ketra-ix-move
else
  bad "cross-fs move (no writable /dev/shm in sandbox)"
fi
# restore moved pieces for later sections
"$BIN" copy "$WORK/s6a" -d "$A/sub" --progress quiet -y >/dev/null 2>&1
printf 'changed content v2\n' > "$A/readme.txt"

# ===========================================================================
say "S7. clipboard ↔ engine (real X11 round-trip under Xvfb)"
if [ -n "$DISPLAY" ]; then
  pkill -f "ketra __clip-server" 2>/dev/null; sleep 0.3
  mkdir -p "$WORK/clip"; cp "$A/readme.txt" "$A/bin.dat" "$WORK/clip/"
  "$BIN" clip put "$WORK/clip/readme.txt" "$WORK/clip/bin.dat" >/dev/null 2>&1
  CLIP_JSON=""
  for i in $(seq 1 30); do
    CLIP_JSON=$("$BIN" clip get --json 2>/dev/null) && [ -n "$CLIP_JSON" ] && break
    sleep 0.2
  done
  [ -n "$CLIP_JSON" ] && ok "clip put → selection owned (get succeeded)" || bad "clip put → selection owned"
  echo "$CLIP_JSON" | python3 -c '
import json,sys
d=json.load(sys.stdin); ps=[p.split("/")[-1] for p in d["paths"]]
sys.exit(0 if d["mode"]=="copy" and set(ps)=={"readme.txt","bin.dat"} else 1)' \
    && ok "clip get --json: mode=copy, both paths" || bad "clip get --json: mode=copy, both paths (got: $CLIP_JSON)"

  "$BIN" clip paste -d "$WORK/paste-copy" --verify inline --progress quiet -y >/dev/null 2>&1
  check $? "clip paste (copy mode) exits 0"
  cmp -s "$WORK/clip/readme.txt" "$WORK/paste-copy/readme.txt" && ok "pasted content identical" || bad "pasted content identical"
  [ -f "$WORK/clip/readme.txt" ] && ok "copy mode keeps sources" || bad "copy mode keeps sources"

  "$BIN" clip put --cut "$WORK/clip/readme.txt" >/dev/null 2>&1
  CUT_JSON=""
  for i in $(seq 1 30); do
    CUT_JSON=$("$BIN" clip get --json 2>/dev/null) && echo "$CUT_JSON" | grep -q '"mode":"cut"' && break
    sleep 0.2
  done
  echo "$CUT_JSON" | grep -q '"mode":"cut"' && ok "clip put --cut → mode=cut" || bad "clip put --cut → mode=cut (got: $CUT_JSON)"
  "$BIN" clip paste -d "$WORK/paste-cut" --verify inline --progress quiet -y >/dev/null 2>&1
  check $? "clip paste (cut mode) exits 0"
  [ -f "$WORK/paste-cut/readme.txt" ] && [ ! -f "$WORK/clip/readme.txt" ] && ok "cut mode MOVED (source removed)" || bad "cut mode MOVED (source removed)"
  cmp -s "$WORK/paste-cut/readme.txt" "$A/readme.txt" && ok "moved content identical" || bad "moved content identical"
else
  bad "clipboard round-trip (no DISPLAY)"
fi

# ===========================================================================
say "S8. shell install → generated scripts ↔ CLI ↔ clipboard ↔ engine"
"$BIN" shell install nemo nautilus caja dolphin thunar pick >"$WORK/shell.log" 2>&1
check $? "shell install exits 0"
NA="$HOME/.local/share/nautilus/scripts"
N_FILES=0
[ -f "$HOME/.local/share/nemo/actions/ketra-copy.nemo_action" ] && N_FILES=$((N_FILES+1))
[ -f "$HOME/.local/share/nemo/actions/ketra-cut.nemo_action" ]  && N_FILES=$((N_FILES+1))
[ -f "$NA/KetraCopy Copy" ] && N_FILES=$((N_FILES+1))
[ -f "$NA/KetraCopy Paste Here" ] && N_FILES=$((N_FILES+1))
[ -f "$HOME/.local/share/caja/scripts/KetraCopy Copy" ] && N_FILES=$((N_FILES+1))
ls "$HOME/.local/share/kio/servicemenus/"*.desktop >/dev/null 2>&1 && N_FILES=$((N_FILES+1))
[ -f "$HOME/.config/Thunar/ketra-uca-snippet.xml" ] && N_FILES=$((N_FILES+1))
[ -x "$HOME/.local/bin/ketra-pick" ] && N_FILES=$((N_FILES+1))
[ -x "$HOME/.local/bin/ketra" ] && N_FILES=$((N_FILES+1))
[ "$N_FILES" -eq 9 ] && ok "all 9 integration artifacts installed" || bad "all 9 integration artifacts installed (found $N_FILES)"
"$BIN" shell status >/dev/null 2>&1; check $? "shell status exits 0"

if [ -n "$DISPLAY" ]; then
  # generated nautilus Copy script → pick helper (KETRA_DEST pre-selects the
  # folder) → ketra copy → engine. Full chain: script ↔ pick ↔ CLI ↔ engine.
  mkdir -p "$WORK/script-copy"
  NAUTILUS_SCRIPT_SELECTED_FILE_PATHS="$A/readme.txt
$A/bin.dat" KETRA_DEST="$WORK/script-copy" \
      "$NA/KetraCopy Copy" >"$WORK/sc.log" 2>&1
  check $? "nautilus Copy script → pick → copy chain exits 0"
  cmp -s "$A/readme.txt" "$WORK/script-copy/readme.txt" && \
      cmp -s "$A/bin.dat" "$WORK/script-copy/bin.dat" && \
      ok "chain copied both files bit-identically" || bad "chain copied both files bit-identically"

  # generated nautilus Test script → ketra test (healthy selection → exit 0)
  NAUTILUS_SCRIPT_SELECTED_FILE_PATHS="$A/sub/data.bin" \
      "$NA/KetraCopy Test (Verify)" >/dev/null 2>&1
  check $? "nautilus Test (Verify) script → ketra test chain works"

  # generated Paste Here script → engine paste into URI-decoded folder
  mkdir -p "$WORK/paste here"
  "$BIN" clip put "$A/bin.dat" >/dev/null 2>&1
  for i in $(seq 1 20); do "$BIN" clip get >/dev/null 2>&1 && break; sleep 0.2; done
  NAUTILUS_SCRIPT_CURRENT_URI="file://$WORK/paste%20here" "$NA/KetraCopy Paste Here" >/dev/null 2>&1
  check $? "nautilus Paste Here script exits 0"
  [ -f "$WORK/paste here/bin.dat" ] && ok "paste-here landed in URI-decoded folder" || bad "paste-here landed in URI-decoded folder"
  cmp -s "$A/bin.dat" "$WORK/paste here/bin.dat" && ok "paste-here content identical" || bad "paste-here content identical"
fi

DESK=$(ls "$HOME/.local/share/kio/servicemenus/"*.desktop 2>/dev/null | head -1)
if [ -n "$DESK" ]; then
  grep -q "^Exec=.*ketra" "$DESK" && ok "dolphin servicemenu Exec references ketra" || bad "dolphin servicemenu Exec references ketra"
  grep -q "Desktop Action" "$DESK" && ok "dolphin servicemenu has action groups" || bad "dolphin servicemenu has action groups"
else
  bad "dolphin servicemenu Exec"
  bad "dolphin servicemenu action groups"
fi
"$HOME/.local/bin/ketra-pick" >/dev/null 2>&1; r=$?
checkn $r "pick helper fails gracefully without a picker (exit $r)"
KETRA_DEST="$WORK" "$HOME/.local/bin/ketra-pick" test "$A/readme.txt" >/dev/null 2>&1
check $? "pick helper honors KETRA_DEST (scripting escape hatch)"

"$BIN" shell uninstall nemo nautilus caja dolphin thunar pick >/dev/null 2>&1
check $? "shell uninstall exits 0"
LEFT=0
[ -e "$HOME/.local/share/nemo/actions/ketra-copy.nemo_action" ] && LEFT=$((LEFT+1))
[ -e "$HOME/.local/share/nautilus/scripts/KetraCopy Copy" ] && LEFT=$((LEFT+1))
[ -e "$HOME/.local/share/caja/scripts/KetraCopy Copy" ] && LEFT=$((LEFT+1))
ls "$HOME/.local/share/kio/servicemenus/"*.desktop >/dev/null 2>&1 && LEFT=$((LEFT+1))
[ -e "$HOME/.local/bin/ketra-pick" ] && LEFT=$((LEFT+1))
[ "$LEFT" -eq 0 ] && ok "uninstall removed integration files" || bad "uninstall removed integration files ($LEFT left)"

# ===========================================================================
say "S9. LAN loopback: mDNS discovery ↔ receive ↔ send ↔ integrity"
"$BIN" receive --dest "$WORK/lan-dest" --alias testbox >"$WORK/recv.log" 2>&1 &
RPID=$!
for i in $(seq 1 50); do grep -q "receiving on" "$WORK/recv.log" && break; sleep 0.2; done
grep -q "receiving on" "$WORK/recv.log" && ok "receive server listening" || bad "receive server listening ($(tail -1 "$WORK/recv.log"))"
sleep 0.5
PEERS_OUT=$("$BIN" peers --wait 8 2>/dev/null)
echo "$PEERS_OUT" | grep -q "testbox" && ok "mDNS loopback discovery found testbox" || bad "mDNS loopback discovery found testbox"
INFO=$(curl -s --max-time 5 http://127.0.0.1:53317/api/localsend/v2/info 2>/dev/null)
echo "$INFO" | grep -q "fingerprint" && ok "info endpoint responds (HTTP/1.1, LocalSend v2 paths)" || bad "info endpoint responds"

if echo "$PEERS_OUT" | grep -q "testbox"; then
  mkdir -p "$WORK/send-src"
  cp "$A/readme.txt" "$A/bin.dat" "$WORK/send-src/"
  python3 -c "
import random; random.seed(3)
open('$WORK/send-src/lan.bin','wb').write(bytes(random.getrandbits(8) for _ in range(1_000_000)))"
  "$BIN" send "$WORK/send-src/readme.txt" "$WORK/send-src/bin.dat" "$WORK/send-src/lan.bin" \
      --to testbox >/dev/null 2>&1
  check $? "send 3 files to testbox exits 0"
  ALLOK=1
  for f in readme.txt bin.dat lan.bin; do
    cmp -s "$WORK/send-src/$f" "$WORK/lan-dest/$f" || ALLOK=0
  done
  [ "$ALLOK" = "1" ] && ok "received files bit-identical (3/3)" || bad "received files bit-identical"
  "$BIN" test "$WORK/lan-dest/lan.bin" --hash sha256 >/dev/null 2>&1 && ok "ketra test healthy on received file" || bad "ketra test healthy on received file"
  "$BIN" send "$WORK/send-src/lan.bin" --to testbox --parallel 4 --chunk 1M >/dev/null 2>&1
  check $? "re-send with --parallel 4 --chunk 1M (accelerated path) exits 0"
  cmp -s "$WORK/send-src/lan.bin" "$WORK/lan-dest/lan.bin" && ok "re-sent file still identical" || bad "re-sent file still identical"
else
  bad "send over loopback (discovery unavailable in this environment)"
fi
kill $RPID 2>/dev/null; wait $RPID 2>/dev/null

# ===========================================================================
say "S10. GUI smoke under Xvfb"
if [ -n "$DISPLAY" ] && [ -x "$GUI" ]; then
  XKBLD="/tmp/xkblibs/usr/lib/x86_64-linux-gnu"
  [ -d "$XKBLD" ] && export LD_LIBRARY_PATH="$XKBLD${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
  LIBGL_ALWAYS_SOFTWARE=1 timeout 8 "$GUI" >"$WORK/gui.out" 2>"$WORK/gui.err"; r=$?
  if [ "$r" = "124" ]; then ok "GUI ran for 8s without crashing (killed by timeout)";
  elif [ "$r" = "0" ]; then ok "GUI exited cleanly";
  else bad "GUI smoke (exit $r): $(tail -2 "$WORK/gui.err" | tr '\n' ' ')"; fi
  grep -qi "panicked at" "$WORK/gui.err" && bad "GUI stderr panic-free" || ok "GUI stderr panic-free"
  unset LD_LIBRARY_PATH
elif [ ! -x "$GUI" ]; then
  bad "GUI smoke (binary missing)"
else
  bad "GUI smoke (no DISPLAY)"
fi

# ===========================================================================
say "S11. artifact cross-consistency (dist ↔ build ↔ deb ↔ sums)"
DIST="$ROOT/dist"
BUILT_SHA=$(SHA "$BIN")
[ "$(SHA "$DIST/linux-x86_64/ketra")" = "$BUILT_SHA" ] && ok "dist/linux-x86_64/ketra == freshly built binary" || bad "dist/linux-x86_64/ketra hash mismatch"
[ "$(SHA "$DIST/linux-x86_64/ketracopy-gui")" = "$(SHA "$GUI")" ] && ok "dist GUI binary == freshly built GUI" || bad "dist GUI binary hash mismatch"

TMPX="$WORK/extract"; mkdir -p "$TMPX"
tar -xzf "$DIST/ketracopy-1.2.0-portable-linux-x86_64.tar.gz" -C "$TMPX" 2>/dev/null
PK=$(find "$TMPX" -name ketra -type f | head -1)
[ -n "$PK" ] && [ "$(SHA "$PK")" = "$BUILT_SHA" ] && ok "portable tar.gz: ketra identical + extractable" || bad "portable tar.gz ketra hash"
"$PK" --version >/dev/null 2>&1 && ok "portable binary runs (--version)" || bad "portable binary runs"

mkdir -p "$TMPX/zip" && (cd "$TMPX/zip" && python3 -m zipfile -e "$DIST/ketracopy-1.2.0-windows-x86_64.zip" . 2>/dev/null)
find "$TMPX/zip" -name ketra.exe -type f | grep -q . && ok "windows zip contains ketra.exe" || bad "windows zip contains ketra.exe"
find "$TMPX/zip" -name ketracopy-gui.exe -type f | grep -q . && ok "windows zip contains ketracopy-gui.exe" || bad "windows zip contains ketracopy-gui.exe"
mkdir -p "$TMPX/mac" && tar -xzf "$DIST/ketracopy-1.2.0-macos-aarch64.tar.gz" -C "$TMPX/mac" 2>/dev/null
find "$TMPX/mac" -name ketra -type f | grep -q . && ok "macos tar.gz contains the CLI binary ($(find "$TMPX/mac" -type f | wc -l) files)" || bad "macos tar.gz contains CLI binary"

# SHA256SUMS.txt entries all match
SUMS_OK=1; SUMS_N=0
(cd "$DIST" && sha256sum -c SHA256SUMS.txt >/dev/null 2>&1) || SUMS_OK=0
SUMS_N=$(grep -c . "$DIST/SHA256SUMS.txt")
[ "$SUMS_OK" = "1" ] && ok "SHA256SUMS.txt: all $SUMS_N entries verify" || bad "SHA256SUMS.txt verification failed"

# .deb payload
mkdir -p "$TMPX/deb" && dpkg-deb -x "$DIST/ketracopy_1.2.0_amd64.deb" "$TMPX/deb" 2>/dev/null
[ -f "$TMPX/deb/usr/bin/ketra" ] && [ "$(SHA "$TMPX/deb/usr/bin/ketra")" = "$BUILT_SHA" ] && ok ".deb payload ketra == built binary" || bad ".deb payload ketra hash"
dpkg-deb -I "$DIST/ketracopy_1.2.0_amd64.deb" 2>/dev/null | grep -qi "ketracopy" && ok ".deb metadata intact" || bad ".deb metadata intact"

# cross-binary manifest compatibility (dist binary ↔ source-built binary)
"$DIST/linux-x86_64/ketra" manifest --root "$A" --out "$WORK/xbin.json" >/dev/null 2>&1; r1=$?
"$BIN" verify --root "$A" --manifest "$WORK/xbin.json" --quiet >/dev/null 2>&1; r2=$?
[ $r1 -eq 0 ] && [ $r2 -eq 0 ] && ok "manifest by dist binary → verify by built binary" || bad "dist→built manifest interop ($r1/$r2)"
"$BIN" manifest --root "$A" --out "$WORK/bxin.json" >/dev/null 2>&1; r1=$?
"$DIST/linux-x86_64/ketra" verify --root "$A" --manifest "$WORK/bxin.json" --quiet >/dev/null 2>&1; r2=$?
[ $r1 -eq 0 ] && [ $r2 -eq 0 ] && ok "manifest by built binary → verify by dist binary" || bad "built→dist manifest interop ($r1/$r2)"

mkdir -p "$TMPX/src" && tar -xzf "$DIST/ketracopy-1.2.0-src.tar.gz" -C "$TMPX/src" 2>/dev/null
find "$TMPX/src" -maxdepth 2 -name Cargo.toml | grep -q . && ok "src tar.gz extracts with workspace root" || bad "src tar.gz workspace root"
(cd "$TMPX/src" && python3 -m zipfile -t "$ROOT/dist/ketracopy-1.2.0-src.zip" >/dev/null 2>&1) && ok "src zip integrity test passes" || bad "src zip integrity"

# ===========================================================================
say "S12. bench (128 MiB)"
BENCH_OUT=$("$BIN" bench --path "$WORK" --size 128M 2>&1); r=$?
check $r "bench exits 0"
W_MBPS=$(echo "$BENCH_OUT" | grep -oE "wrote .* [0-9.]+ MiB/s" | grep -oE "[0-9.]+ MiB/s" | grep -oE "[0-9.]+" | head -1)
R_MBPS=$(echo "$BENCH_OUT" | grep -oE "read  .* [0-9.]+ MiB/s" | grep -oE "[0-9.]+ MiB/s" | grep -oE "[0-9.]+" | head -1)
[ -n "$W_MBPS" ] && ok "bench write throughput: ${W_MBPS} MiB/s" || bad "bench write throughput parse"
[ -n "$R_MBPS" ] && ok "bench read throughput: ${R_MBPS} MiB/s" || bad "bench read throughput parse"

# ===========================================================================
printf '\n============ INTEROP SUMMARY ============\n'
printf '  PASS: %d  FAIL: %d\n' "$PASS" "$FAIL"
[ "$FAIL" -eq 0 ] || exit 1
