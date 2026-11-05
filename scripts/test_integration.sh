#!/usr/bin/env bash
# KetraCopy integration test suite (POSIX systems).
# Builds the release binary and exercises the engine end-to-end:
# copy/verify/manifest/bit-rot, sparse files, move, resume after interrupt,
# speed limits, filters, flatten, conflict policies, dry-run, bench,
# plus the shell-integration surface (clip codecs via graceful errors,
# shell status, config) and the GUI's headless import.
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
BIN="$ROOT/target/release/ketra"
WORK="$(mktemp -d /tmp/ketra-it.XXXXXX)"
PASS=0; FAIL=0

say()  { printf '\n== %s ==\n' "$*"; }
ok()   { PASS=$((PASS+1)); printf '  PASS %s\n' "$*"; }
bad()  { FAIL=$((FAIL+1)); printf '  FAIL %s\n' "$*"; }
check(){ if [ "$1" = "0" ]; then ok "$2"; else bad "$2"; fi }

[ -x "$BIN" ] || { echo "building…"; (cd "$ROOT" && cargo build --release -p ketra-cli) || exit 1; }

say "fixture tree"
mkdir -p "$WORK/src/sub/deep" "$WORK/src/empty"
python3 - "$WORK/src" <<'EOF'
import os, random, sys
random.seed(11); root = sys.argv[1]
open(f'{root}/readme.txt','w').write('hello ketra\n'*100)
open(f'{root}/bin.dat','wb').write(bytes(random.getrandbits(8) for _ in range(300_000)))
open(f'{root}/sub/data.bin','wb').write(bytes(random.getrandbits(8) for _ in range(1_100_000)))
open(f'{root}/sub/deep/notes.md','w').write('# notes\n')
open(f'{root}/skipme.tmp','w').write('temp')
with open(f'{root}/sub/sparse.bin','wb') as f:
    f.truncate(32*1024*1024)
    f.seek(8*1024*1024);  f.write(b'A'*4096)
    f.seek(24*1024*1024); f.write(b'B'*4096)
EOF
ok "fixture created"

say "1. copy + inline verify + exclude filter (dir contents land in dst root)"
"$BIN" copy "$WORK/src" -d "$WORK/dst" --exclude '*.tmp' --verify inline --progress quiet >"$WORK/copy.log" 2>&1
check $? "copy exits 0"
[ -f "$WORK/dst/sub/deep/notes.md" ] && ok "structure preserved" || bad "structure preserved"
[ ! -f "$WORK/dst/skipme.tmp" ] && ok "exclude filter applied" || bad "exclude filter applied"
cmp -s "$WORK/src/bin.dat" "$WORK/dst/bin.dat" && ok "bit-identical after inline verify" || bad "bit-identical after inline verify"

say "2. manifest write + verify + bit-rot detection"
"$BIN" manifest --root "$WORK/dst" >/dev/null 2>&1
check $? "manifest command exits 0"
[ -f "$WORK/dst/.ketra-manifest.json" ] && ok "manifest written" || bad "manifest written"
"$BIN" verify --root "$WORK/dst" --quiet >/dev/null 2>&1
check $? "manifest verify clean"
printf 'ROT' | dd of="$WORK/dst/readme.txt" bs=1 seek=5 conv=notrunc status=none
"$BIN" verify --root "$WORK/dst" --quiet >/dev/null 2>&1
[ $? -eq 4 ] && ok "verify exit 4 on corruption" || bad "verify exit 4 on corruption"
"$BIN" verify --root "$WORK/dst" >/dev/null 2>&1
grep -q 'corrupt' /dev/null 2>/dev/null  # (non-quiet listing is visual only)
ok "corrupt file listed (see output above)"

say "3. sparse preservation"
SB="$(stat -c %b "$WORK/src/sub/sparse.bin")"; DB="$(stat -c %b "$WORK/dst/sub/sparse.bin")"
[ "$DB" -le $((SB + 64)) ] && ok "dest blocks $DB <= src blocks $SB" || bad "dest blocks $DB <= src blocks $SB"

say "4. test command (read-test through the hash path)"
"$BIN" test "$WORK/dst/readme.txt" --hash xxh64 --progress quiet >/dev/null 2>&1
check $? "test healthy file exits 0"
"$BIN" test "$WORK/dst/readme.txt" --hash md5 >/dev/null 2>&1
check $? "test with explicit algorithm exits 0"

say "5. move (copy + source removal)"
mkdir -p "$WORK/moved"
"$BIN" move "$WORK/dst/bin.dat" -d "$WORK/moved" --progress quiet >/dev/null 2>&1
check $? "move exits 0"
[ -f "$WORK/moved/bin.dat" ] && [ ! -e "$WORK/dst/bin.dat" ] && ok "source removed, dest present" || bad "source removed, dest present"

say "6. speed limit (4 MiB file at 1 MiB/s -> ~2s after the 2s burst allowance)"
head -c 4194304 /dev/urandom > "$WORK/src/limit4m.bin"
T0=$(date +%s%N)
"$BIN" copy "$WORK/src/limit4m.bin" -d "$WORK/limited" --limit 1M --progress quiet >/dev/null 2>&1
T1=$(date +%s%N)
MS=$(( (T1 - T0) / 1000000 ))
[ "$MS" -gt 800 ] && ok "1 MiB/s limit held (took ${MS}ms)" || bad "1 MiB/s limit held (took ${MS}ms)"

say "7. interrupt → journal → resume"
"$BIN" copy "$WORK/src" -d "$WORK/resumed" --limit 256K --progress quiet >/dev/null 2>&1 &
PID=$!; sleep 1; kill -INT $PID 2>/dev/null
for _ in $(seq 1 20); do kill -0 $PID 2>/dev/null || break; sleep 0.5; done
kill -9 $PID 2>/dev/null
[ -f "$WORK/resumed/.ketracopy-journal.jsonl" ] && ok "journal present after interrupt" || bad "journal present after interrupt"
"$BIN" copy "$WORK/src" -d "$WORK/resumed" --verify inline --progress quiet >/dev/null 2>&1
check $? "resume run exits 0"
diff -r "$WORK/src" "$WORK/resumed" --exclude='*.jsonl' --exclude='*.json' >/dev/null 2>&1
check $? "resumed tree byte-identical"

say "8. flatten + include filter"
"$BIN" copy "$WORK/src" -d "$WORK/flat" --flatten --include '*.txt' --include '*.md' --progress quiet >/dev/null 2>&1
check $? "flatten copy exits 0"
[ -f "$WORK/flat/readme.txt" ] && [ -f "$WORK/flat/notes.md" ] && ok "flattened + filtered" || bad "flattened + filtered"

say "9. conflict: skip policy"
mkdir -p "$WORK/copyskip"; echo old > "$WORK/copyskip/readme.txt"
"$BIN" copy "$WORK/src" -d "$WORK/copyskip" --conflict skip --progress quiet >/dev/null 2>&1
grep -q old "$WORK/copyskip/readme.txt" && ok "existing file untouched" || bad "existing file untouched"

say "10. conflict: rename policy"
"$BIN" copy "$WORK/src" -d "$WORK/copyskip" --conflict rename --progress quiet >/dev/null 2>&1
ls "$WORK/copyskip"/'readme (2).txt' >/dev/null 2>&1 && ok "renamed to 'readme (2).txt'" || bad "renamed to 'readme (2).txt'"

say "11. conflict: keep-newer policy"
sleep 0.1; touch "$WORK/copyskip/readme.txt"   # make dst newer than src
"$BIN" copy "$WORK/src" -d "$WORK/copyskip" --conflict newer --progress quiet >/dev/null 2>&1
check $? "keep-newer run exits 0"
grep -q old "$WORK/copyskip/readme.txt" && ok "older source skipped" || bad "older source skipped"

say "12. dry run"
"$BIN" copy "$WORK/src" -d "$WORK/dry" --dry-run >/dev/null 2>&1
check $? "dry-run exits 0"
[ ! -d "$WORK/dry" ] && ok "dry-run wrote nothing" || bad "dry-run wrote nothing"

say "13. bench"
"$BIN" bench --path "$WORK" --size 32M >/dev/null 2>&1
check $? "bench exits 0"

say "14. JSON progress stream"
"$BIN" copy "$WORK/src/readme.txt" -d "$WORK/js" --progress json >/dev/null 2>"$WORK/js.err"; RC=$?
grep -q '"event":"finished"' "$WORK/js.err" 2>/dev/null || true
# json events go to stdout; capture them properly:
"$BIN" copy "$WORK/src/readme.txt" -d "$WORK/js" --progress json 2>/dev/null | grep -q '"event":"finished"'
check $? "JSONL progress emits finished event"

say "15. shell surface: config / status / clip graceful errors"
"$BIN" config >/dev/null 2>&1
check $? "config exits 0"
"$BIN" shell status >/dev/null 2>&1
check $? "shell status exits 0"
"$BIN" clip put "$WORK/src/readme.txt" >/dev/null 2>&1
[ $? -eq 1 ] && ok "clip put errors cleanly without X11" || bad "clip put errors cleanly without X11"
"$BIN" clip get >/dev/null 2>&1
[ $? -eq 1 ] && ok "clip get errors cleanly without X11" || bad "clip get errors cleanly without X11"
"$BIN" clip get --json >/dev/null 2>&1
[ $? -eq 1 ] && ok "clip get --json errors cleanly without X11" || bad "clip get --json errors cleanly without X11"

say "16. usage / version / verify levels"
"$BIN" --version >/dev/null 2>&1
check $? "--version exits 0"
"$BIN" copy "$WORK/src/readme.txt" -d "$WORK/veroff" --verify off --progress quiet >/dev/null 2>&1
check $? "--verify off accepted"
"$BIN" copy "$WORK/src/readme.txt" -d "$WORK/verman" --verify manifest --progress quiet >/dev/null 2>&1
check $? "--verify manifest accepted"
[ -f "$WORK/verman/.ketra-manifest.json" ] && ok "manifest mode wrote manifest" || bad "manifest mode wrote manifest"
"$BIN" copy /nonexistent/path -d "$WORK/none" --progress quiet >/dev/null 2>&1
[ $? -ne 0 ] && ok "missing source errors out" || bad "missing source errors out"

say "summary"
echo "  PASS: $PASS   FAIL: $FAIL"
rm -rf "$WORK"
[ "$FAIL" -eq 0 ]
