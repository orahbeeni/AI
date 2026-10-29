#!/usr/bin/env bash
# End-to-end checks against a paired server/client in work dir $2 (see run_e2e.sh), using
# file-backed clipboards and logged input so nothing touches the real desktop.
set -u
BIN=$1; T=$2
export WIFISHARE_FAKE_INPUT=1
pass=0; fail=0
ok()   { echo "  PASS $1"; pass=$((pass+1)); }
bad()  { echo "  FAIL $1"; fail=$((fail+1)); }
# wait_until <seconds> <command...>
wait_until() { local t=$1; shift; for _ in $(seq $((t*10))); do "$@" && return 0; sleep 0.1; done; return 1; }
log_has() { grep -q -- "$2" "$1"; }

start_server() { WIFISHARE_FAKE_AUDIO=$T/SA WIFISHARE_FAKE_CLIPBOARD=$T/SC $BIN --config-dir $T/S server --no-input >>$T/server.log 2>&1 & SPID=$!; }
start_client() { WIFISHARE_FAKE_AUDIO=$T/CA WIFISHARE_FAKE_CLIPBOARD=$T/CC $BIN --config-dir $T/C client >>$T/client.log 2>&1 & CPID=$!; }
connected_count() { grep -c "connected to test-S" $T/client.log; }
more_connections() { [ "$(connected_count)" -gt "$before" ]; }

: >$T/server.log; : >$T/client.log
start_server; sleep 0.5; start_client
echo "1. connect"
wait_until 5 log_has $T/client.log "connected to test-S" && ok "client connected" || bad "client connected"

echo "1b. sound, client -> server (a 440 Hz test tone)"
sound_ok() {
  python3 -I - "$T/SA/received.raw" <<'PY'
import struct, sys
data = open(sys.argv[1], "rb").read()
left = struct.unpack("<%dh" % (len(data) // 2), data[: len(data) // 2 * 2])[::2]
last = left[-48000:]                       # the most recent second
loud = sum(1 for v in last if abs(v) > 1000)
crossings = sum(1 for a, b in zip(last, last[1:]) if (a < 0) != (b < 0))
print(f"     {len(left) / 48000:.1f} s played, {crossings / 2:.0f} Hz, {loud / len(last):.0%} non-silent")
sys.exit(0 if len(left) >= 72000 and 430 <= crossings / 2 <= 450 and loud > 0.5 * len(last) else 1)
PY
}
sleep 2.5
sound_ok && ok "tone arrived and plays in real time" || bad "sound"

echo "2. text, server -> client"
rm -f $T/CC/text; echo -n "hello from the server" >$T/SC/text
wait_until 3 grep -qsx "hello from the server" $T/CC/text && ok "text arrived" || bad "text arrived"

echo "3. text, client -> server"
sleep 0.3; echo -n "hello back" >$T/CC/text
wait_until 3 grep -qx "hello back" $T/SC/text && ok "text arrived" || bad "text arrived"

echo "4. large text goes over a bulk connection"
python3 -c "print('x' * 400000, end='')" >$T/big.txt; cp $T/big.txt $T/SC/text
wait_until 5 cmp -s $T/big.txt $T/CC/text && ok "400 KB text arrived intact" || bad "400 KB text"

echo "5. files and folders, client -> server"
rm -rf $T/src; mkdir -p "$T/src/My Folder/sub"; echo one >"$T/src/My Folder/a.txt"; head -c 3000000 /dev/urandom >"$T/src/My Folder/sub/blob.bin"
chmod +x "$T/src/My Folder/a.txt"; echo solo >$T/src/note.md
printf '%s\n%s\n' "$T/src/My Folder" "$T/src/note.md" >$T/CC/files
if wait_until 10 test -s $T/SC/files; then
  dest=$(head -1 $T/SC/files); note=$(sed -n 2p $T/SC/files)
  diff -r "$T/src/My Folder" "$dest" >/dev/null && cmp -s $T/src/note.md "$note" && ok "files arrived intact ($dest)" || bad "file contents differ"
  test -x "$dest/a.txt" && ok "exec bit kept" || bad "exec bit"
else bad "files arrived"; fi

echo "6. server frozen (like leaving Wi-Fi range) and back"
before=$(connected_count)
kill -STOP $SPID; sleep 7
log_has $T/client.log "silent for" && ok "client noticed the dead link" || bad "dead link noticed"
t0=$(date +%s.%N); kill -CONT $SPID
if wait_until 10 more_connections; then
  echo "     reconnected $(python3 -c "print(round($(date +%s.%N)-$t0,2))")s after the server came back"; ok "reconnected"
else bad "reconnected"; fi
sleep 0.5
echo -n "after the outage" >$T/SC/text
wait_until 3 grep -qx "after the outage" $T/CC/text && ok "clipboard works after reconnect" || bad "clipboard after reconnect"

echo "7. server restart; copy made while the client was away is replayed"
kill -INT $SPID; wait $SPID 2>/dev/null
kill $CPID; wait $CPID 2>/dev/null
start_server; sleep 0.5
echo -n "copied while you were away" >$T/SC/text; sleep 0.5
start_client
wait_until 6 grep -qx "copied while you were away" $T/CC/text && ok "replayed on connect" || bad "replay on connect"

echo "8. discovery: no address known, server found by its beacons"
kill $CPID; wait $CPID 2>/dev/null
sed -i '/^last_addr/d' $T/C/peers.toml
before=$(connected_count)
start_client
wait_until 6 more_connections && ok "found via beacon: $(grep 'connected to test-S' $T/client.log | tail -1 | sed 's/.* at //')" || bad "discovery"

echo "9. shared folders (the client browses and copies with the server's shares)"
FILES="$BIN --config-dir $T/C files"
echo hello >"$T/docs/My Folder/a.txt"; head -c 200000 /dev/urandom >"$T/docs/My Folder/sub/blob.bin"; echo top >"$T/docs/top.txt"
$FILES ls 2>&1 | grep -q "^Docs" && $FILES ls 2>&1 | grep -q "^Inbox" && ok "lists the shares" || bad "lists the shares"
$FILES ls Docs 2>&1 | grep -q "My Folder/" && ok "lists a share's contents" || bad "lists a share's contents"
rm -rf $T/got; mkdir $T/got
$FILES get "Docs/My Folder" $T/got >/dev/null 2>&1 && diff -r "$T/docs/My Folder" "$T/got/My Folder" >/dev/null && ok "folder downloaded intact" || bad "folder download"
$FILES get Docs/top.txt $T/got >/dev/null 2>&1; $FILES get Docs/top.txt $T/got >/dev/null 2>&1
cmp -s $T/docs/top.txt "$T/got/top (2).txt" && ok "a second copy gets a new name instead of overwriting" || bad "no overwrite"
$FILES put Inbox "$T/src/My Folder" $T/src/note.md >/dev/null 2>&1 && diff -r "$T/src/My Folder" "$T/inbox/My Folder" >/dev/null && cmp -s $T/src/note.md $T/inbox/note.md && ok "folder and file uploaded intact" || bad "upload"
test -x "$T/inbox/My Folder/a.txt" && ok "exec bit kept" || bad "exec bit on upload"
$FILES put Docs $T/src/note.md 2>&1 | grep -qi "read-only" && ok "read-only share refuses uploads" || bad "read-only refusal"
$FILES get "Docs/../../etc/passwd" $T/got 2>&1 | grep -qi "outside\|no such\|unsafe\|refusing" && ok "paths outside the share are refused" || bad "path escape refusal"

echo "10. a big copy interrupted by the server going away carries on afterwards"
head -c 1500000000 /dev/zero | tr '\0' 'q' >"$T/docs/big.bin"
rm -rf $T/got2; mkdir $T/got2
$FILES get Docs/big.bin $T/got2 >$T/get.log 2>&1 & GPID=$!
part_started() { [ "$(stat -c %s $T/got2/big.bin.wifishare-part 2>/dev/null || echo 0)" -gt 20000000 ]; }
wait_until 20 part_started
kill -INT $SPID; wait $SPID 2>/dev/null
cut_at=$(stat -c %s $T/got2/big.bin.wifishare-part 2>/dev/null || echo 0)
sleep 1.5
start_server
wait $GPID && cmp -s $T/docs/big.bin $T/got2/big.bin && ok "finished after the server came back (cut at ${cut_at} bytes)" || { bad "interrupted copy"; cat $T/get.log; tail -5 $T/server.log; }
rm -f $T/docs/big.bin

kill $CPID $SPID 2>/dev/null; wait 2>/dev/null
echo "passed $pass, failed $fail"
[ "$fail" -eq 0 ]
