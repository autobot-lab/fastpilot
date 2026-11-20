#!/usr/bin/env bash
# One local command that runs every check before a release (not CI): each milestone and each
# release is tested the same way, on a Mac with Chrome, an Android emulator and an iOS simulator.
#
#   scripts/release-check.sh                  every check
#   scripts/release-check.sh --skip ios       mark checks as skipped (code, unit, web, android, ios)
#   scripts/release-check.sh --strict         a skipped check also means NOT READY
#
# Runs, in order: code checks (cargo clippy -D warnings), unit tests, the web e2e suites, the
# Android suites on one emulator, the iOS suites on the booted simulator. Then it prints one
# summary: each check passed / failed / skipped with the reason, and READY TO RELEASE or NOT READY.
# A missing device is a skipped check with its reason, never a pass. Exit 0 only when READY.
#
# Android runs on RELEASE_CHECK_ANDROID_SERIAL (default emulator-5556) and on no other device;
# emulator-5554 is never used (it is the emulator kept for hands-on work).
# The tests run with a throwaway HOME and FASTPILOT_KEYSTORE=file, so your ~/.fastpilot and the
# Keychain are not read or changed. The repo's .fastpilot/ (project memory, recordings) is moved
# aside for the run and put back afterwards; the run's own .fastpilot/ is kept next to the logs.
# Code checks, unit tests and web tests need no device: if one is skipped the result is NOT READY.
# Android and iPhone may be skipped (the summary says why); --strict makes that NOT READY too.
# Logs, results.tsv and release-check-report.html go to target/release-check/<time>/; the summary
# also says what changed since the previous run.
set -uo pipefail
cd "$(dirname "$0")/.."
ROOT="$(pwd)"

usage() { sed -n '2,19p' "$0" | sed 's/^# \{0,1\}//'; }
SKIP=","
STRICT=0
while [ $# -gt 0 ]; do
  case "$1" in
    --skip) [ $# -ge 2 ] || { echo "--skip needs a list, e.g. --skip android,ios" >&2; exit 2; }
            SKIP="$SKIP$2,"; shift 2 ;;
    --skip=*) SKIP="$SKIP${1#--skip=},"; shift ;;
    --strict) STRICT=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown option: $1 (see --help)" >&2; exit 2 ;;
  esac
done

SERIAL="${RELEASE_CHECK_ANDROID_SERIAL:-emulator-5556}"
if [ "$SERIAL" = "emulator-5554" ]; then
  echo "emulator-5554 is never used by the release check; start another emulator (e.g. -port 5556)" >&2
  exit 2
fi

KEYS="code unit web android ios"
NAMES=("Code checks" "Unit tests" "Web tests" "Android tests" "iPhone tests")
STATUS=(); DETAIL=(); SECS=()
for w in $(echo "$SKIP" | tr ',' ' '); do
  case " $KEYS " in *" $w "*) ;; *) echo "unknown check in --skip: $w (use: $KEYS)" >&2; exit 2 ;; esac
done

if [ -t 1 ]; then G=$'\033[32m'; R=$'\033[31m'; Y=$'\033[33m'; B=$'\033[1m'; N=$'\033[0m'; else G=""; R=""; Y=""; B=""; N=""; fi

STAMP="$(date +%Y%m%d-%H%M%S)"
OUT="${CARGO_TARGET_DIR:-$ROOT/target}/release-check/$STAMP"
mkdir -p "$OUT"
SHA="$(git rev-parse --short HEAD 2>/dev/null || echo unknown)"
BRANCH="$(git rev-parse --abbrev-ref HEAD 2>/dev/null || echo unknown)"
DIRTY=""
if [ -n "$(git status --porcelain --untracked-files=no 2>/dev/null)" ]; then DIRTY=" (uncommitted changes)"; fi
printf '%s\t%s\t%s\n' "$SHA" "$BRANCH" "$(date '+%Y-%m-%d %H:%M')" > "$OUT/meta.tsv"

# Isolation: real toolchains, throwaway state.
REAL_HOME="$HOME"
export CARGO_HOME="${CARGO_HOME:-$REAL_HOME/.cargo}" RUSTUP_HOME="${RUSTUP_HOME:-$REAL_HOME/.rustup}"
export ANDROID_HOME="${ANDROID_HOME:-$REAL_HOME/Library/Android/sdk}"
export PATH="$CARGO_HOME/bin:$PATH"
SANDBOX="$(mktemp -d "${TMPDIR:-/tmp}/fastpilot-release-check.XXXXXX")"
mkdir -p "$SANDBOX/home"
# `adb emu` (the SMS test) authenticates to the emulator console with this token.
[ -f "$REAL_HOME/.emulator_console_auth_token" ] && cp "$REAL_HOME/.emulator_console_auth_token" "$SANDBOX/home/"
for v in $(env | sed -n 's/^\(FASTPILOT_[A-Z0-9_]*\)=.*/\1/p'); do
  [ "$v" = FASTPILOT_CHROME ] || unset "$v"
done
unset ANDROID_SERIAL
# Leftovers in .fastpilot/ (memory, quarantine) change test results: start from none.
STATE="$ROOT/.fastpilot"; MOVED=0
if [ -e "$STATE" ]; then mv "$STATE" "$OUT/fastpilot-before" && MOVED=1; fi
cleanup() {
  [ -e "$STATE" ] && mv "$STATE" "$OUT/fastpilot-run"
  [ "$MOVED" -eq 1 ] && mv "$OUT/fastpilot-before" "$STATE"
  rm -rf "$SANDBOX"
}
trap cleanup EXIT
INTERRUPTED=0
trap 'INTERRUPTED=1' INT

# Runs "$@" with the test environment, output appended to $LOG; returns its exit code.
tenv() {
  HOME="$SANDBOX/home" FASTPILOT_KEYSTORE=file FASTPILOT_CONFIG="$SANDBOX/home/.fastpilot/config.toml" "$@" >>"$LOG" 2>&1
}

# Totals over every `test result:` line in $LOG: sets PASSED FAILED RESULTS SKIPPED FAILS.
count() {
  set -- $(awk '/^test result:/ { n++; for (i = 2; i <= NF; i++) { if ($i ~ /^passed;/) p += $(i-1); if ($i ~ /^failed;/) f += $(i-1) } }
               END { print p+0, f+0, n+0 }' "$LOG")
  PASSED=$1; FAILED=$2; RESULTS=$3
  SKIPPED=$(grep -o 'skipping[ :]' "$LOG" | wc -l | tr -d ' ')
  FAILS=$(awk '/^failures:$/ { f = 1; next } /^test result:/ { f = 0 } f && /^    [A-Za-z0-9_:]+$/ { sub(/^ +/, ""); print }' "$LOG" | sort -u)
}

list() { # "a b c d e" -> "a, b, c (+2 more)"
  echo "$1" | awk 'NF { n++; if (n <= 3) s = s (n > 1 ? ", " : "") $0 } END { if (n > 3) s = s " (+" n - 3 " more)"; print s }'
}

# Turns the cargo test totals into a status for check $1. $2 = exit code. $3 = 1 when the device
# was found, so tests that still skip mean something is wrong (not a pass).
judge() {
  local i=$1 rc=$2 device=${3:-0} ran
  count
  ran=$((PASSED + FAILED))
  if [ "$FAILED" -gt 0 ]; then
    STATUS[$i]=failed; DETAIL[$i]="$FAILED of $ran failed: $(list "$FAILS")"
  elif [ "$rc" -ne 0 ] && [ "$INTERRUPTED" -eq 1 ]; then
    STATUS[$i]=failed; DETAIL[$i]="interrupted"
  elif [ "$rc" -ne 0 ] || [ "$RESULTS" -eq 0 ]; then
    STATUS[$i]=failed; DETAIL[$i]="did not build or run (exit $rc), see $(basename "$LOG")"
  elif [ "$device" -eq 1 ] && [ "$SKIPPED" -ge "$ran" ]; then
    STATUS[$i]=failed; DETAIL[$i]="0 of $ran ran: every test skipped although the device is ready, see $(basename "$LOG")"
  elif [ "$SKIPPED" -gt 0 ]; then
    STATUS[$i]=passed; DETAIL[$i]="$((ran - SKIPPED)) of $ran passed, $SKIPPED skipped inside the suite"
  elif [ "$i" -eq 1 ]; then
    STATUS[$i]=passed; DETAIL[$i]="$PASSED passed"
  else
    STATUS[$i]=passed; DETAIL[$i]="$PASSED of $ran passed"
  fi
}

skip() { STATUS[$1]=skipped; DETAIL[$1]="skipped: $2"; }

check_code() {
  tenv cargo clippy --workspace --all-targets --locked -- -D warnings
  local rc=$? n
  if [ "$rc" -eq 0 ]; then STATUS[0]=passed; DETAIL[0]="passed"; return; fi
  n=$(grep -cE '^error(\[|:)' "$LOG"); STATUS[0]=failed
  DETAIL[0]="failed: $n clippy error(s), see $(basename "$LOG")"
  [ "$INTERRUPTED" -eq 1 ] && DETAIL[0]="interrupted"
}

check_unit() {
  tenv cargo test --release --locked --workspace --lib --bins --no-fail-fast
  judge 1 $?
}

# Every suite in bin/fastpilot/tests except the device ones. The Android demo test in demo_cli
# needs an emulator, so it runs with the Android check instead.
ANDROID_DEMO=demo_android_installs_the_app_and_its_login_runs
WEB_SUITES=""
for f in bin/fastpilot/tests/*.rs; do
  s="$(basename "$f" .rs)"
  case "$s" in android|ios|real_android|real_ios|ui_takeover_device) ;; *) WEB_SUITES="$WEB_SUITES $s" ;; esac
done

check_web() {
  local args=() s
  for s in $WEB_SUITES; do args+=(--test "$s"); done
  tenv cargo test --release --locked -p fastpilot --no-fail-fast "${args[@]}" -- --nocapture --skip "$ANDROID_DEMO"
  judge 2 $?
}

check_android() {
  local adb="$ANDROID_HOME/platform-tools/adb" rc
  [ -x "$adb" ] || { skip 3 "no Android emulator (no adb: set ANDROID_HOME to the Android SDK)"; return; }
  if [ "$("$adb" -s "$SERIAL" get-state 2>/dev/null)" != device ]; then
    skip 3 "no Android emulator ($SERIAL is not running; start one with -port ${SERIAL#emulator-})"; return
  fi
  echo "== agent-android/build.sh" >>"$LOG"
  agent-android/build.sh >>"$LOG" 2>&1 || { STATUS[3]=failed; DETAIL[3]="agent-android/build.sh failed, see $(basename "$LOG")"; return; }
  echo "== tests on $SERIAL" >>"$LOG"
  tenv env FASTPILOT_ANDROID_SERIAL="$SERIAL" ANDROID_SERIAL="$SERIAL" \
    cargo test --release --locked -p fastpilot --no-fail-fast --test android -- --test-threads=1 --nocapture
  rc=$?
  tenv env FASTPILOT_ANDROID_SERIAL="$SERIAL" ANDROID_SERIAL="$SERIAL" \
    cargo test --release --locked -p fastpilot --test ui_takeover_device -- --exact take_over_android --nocapture
  [ $? -ne 0 ] && rc=1
  tenv env FASTPILOT_ANDROID_SERIAL="$SERIAL" ANDROID_SERIAL="$SERIAL" \
    cargo test --release --locked -p fastpilot --test demo_cli -- --exact "$ANDROID_DEMO" --nocapture
  [ $? -ne 0 ] && rc=1
  judge 3 "$rc" 1
}

check_ios() {
  local udid rc
  command -v xcrun >/dev/null 2>&1 || { skip 4 "no simulator (needs macOS with Xcode)"; return; }
  udid="$(xcrun simctl list devices booted 2>/dev/null | sed -n 's/.*(\([0-9A-F-]\{36\}\)) (Booted).*/\1/p' | head -1)"
  [ -n "$udid" ] || { skip 4 "no simulator (none booted; xcrun simctl boot \"iPhone 16\")"; return; }
  echo "== ios-runner/build.sh" >>"$LOG"
  ios-runner/build.sh >>"$LOG" 2>&1 || { STATUS[4]=failed; DETAIL[4]="ios-runner/build.sh failed, see $(basename "$LOG")"; return; }
  echo "== tests on simulator $udid" >>"$LOG"
  tenv env FASTPILOT_IOS_DEVICE="$udid" \
    cargo test --release --locked -p fastpilot --no-fail-fast --test ios -- --test-threads=1 --nocapture
  rc=$?
  tenv env FASTPILOT_IOS_DEVICE="$udid" \
    cargo test --release --locked -p fastpilot --test ui_takeover_device -- --exact take_over_ios_simulator --nocapture
  [ $? -ne 0 ] && rc=1
  judge 4 "$rc" 1
}

fmt_time() { printf '%dm%02ds' $(($1 / 60)) $(($1 % 60)); }
mark() { case "$1" in passed) printf '%s✓%s' "$G" "$N" ;; failed) printf '%s✗%s' "$R" "$N" ;; *) printf '%s-%s' "$Y" "$N" ;; esac; }

echo "${B}Release check${N} · $BRANCH @ $SHA$DIRTY · logs in ${OUT#$ROOT/}/"
i=0
for k in $KEYS; do
  LOG="$OUT/$k.log"; : >"$LOG"; SECS[$i]=0
  if [ "$INTERRUPTED" -eq 1 ]; then skip $i "interrupted"
  else
    case "$SKIP" in
      *",$k,"*) skip $i "--skip $k" ;;
      *)
        printf '  %-14s running …\n' "${NAMES[$i]}"
        t0=$SECONDS
        "check_$k"
        SECS[$i]=$((SECONDS - t0))
        printf '  %-14s %s %s (%s)\n' "${NAMES[$i]}" "$(mark "${STATUS[$i]}")" "${STATUS[$i]}" "$(fmt_time "${SECS[$i]}")" ;;
    esac
  fi
  printf '%s\t%s\t%s\t%s\n' "$k" "${STATUS[$i]}" "${DETAIL[$i]}" "${SECS[$i]}" >>"$OUT/results.tsv"
  i=$((i + 1))
done

nfail=0; nskip=0; failed_names=""; skipped_names=""; nobase=""
for i in 0 1 2 3 4; do
  case "${STATUS[$i]}" in
    failed) nfail=$((nfail + 1)); failed_names="$failed_names${failed_names:+, }${NAMES[$i]}" ;;
    skipped) nskip=$((nskip + 1)); skipped_names="$skipped_names${skipped_names:+, }${NAMES[$i]}"
             [ "$i" -le 2 ] && nobase="$nobase${nobase:+, }${NAMES[$i]}" ;;
  esac
done
plural() { [ "$1" -eq 1 ] && echo "1 check" || echo "$1 checks"; }
if [ "$INTERRUPTED" -eq 1 ]; then READY=0; RESULT="NOT READY. Interrupted before every check ran."
elif [ "$nfail" -gt 0 ]; then READY=0; RESULT="NOT READY. $(plural $nfail) failed: $failed_names."
elif [ -n "$nobase" ]; then READY=0; RESULT="NOT READY. Skipped, but needed for every release: $nobase."
elif [ "$nskip" -gt 0 ] && [ "$STRICT" -eq 1 ]; then READY=0; RESULT="NOT READY. --strict and $(plural $nskip) skipped: $skipped_names."
elif [ "$nskip" -gt 0 ]; then READY=1; RESULT="READY TO RELEASE, with $(plural $nskip) skipped."
else READY=1; RESULT="READY TO RELEASE."
fi

SUMMARY="$OUT/summary.txt"
{
  echo "Release check · $BRANCH @ $SHA$DIRTY · $(date '+%Y-%m-%d %H:%M')"
  for i in 0 1 2 3 4; do
    case "${STATUS[$i]}" in passed) m="✓" ;; failed) m="✗" ;; *) m="-" ;; esac
    printf '%s %-14s %s\n' "$m" "${NAMES[$i]}" "${DETAIL[$i]}"
  done
  echo "Result: $RESULT"
} >"$SUMMARY"

# What changed since the previous run (newest other directory with results).
CHANGES=""
PREV=""
for d in $(ls -1d "$(dirname "$OUT")"/*/ 2>/dev/null | sort -r); do
  d="${d%/}"
  [ "$d" = "$OUT" ] && continue
  [ -f "$d/results.tsv" ] && [ -f "$d/meta.tsv" ] && { PREV="$d"; break; }
done
if [ -n "$PREV" ]; then
  prev_meta="$(awk -F'\t' '{ print $1 ", " $3 }' "$PREV/meta.tsv")"
  i=0
  for k in $KEYS; do
    old="$(awk -F'\t' -v k="$k" '$1 == k { print $3 }' "$PREV/results.tsv")"
    new="${DETAIL[$i]}"
    [ -n "$old" ] && [ "$old" != "$new" ] && CHANGES="$CHANGES  ${NAMES[$i]}: $old → $new"$'\n'
    i=$((i + 1))
  done
  if [ -n "$CHANGES" ]; then CHANGES="Since the last run ($prev_meta):"$'\n'"$CHANGES"
  else CHANGES="No change since the last run ($prev_meta)."$'\n'; fi
fi

html() { sed -e 's/&/\&amp;/g' -e 's/</\&lt;/g' -e 's/>/\&gt;/g'; }
REPORT="$OUT/release-check-report.html"
{
  echo "<!doctype html><meta charset=utf-8><title>fastpilot release check $SHA</title>"
  echo "<style>body{font:15px -apple-system,sans-serif;margin:2em;max-width:60em}td,th{padding:.4em .8em;text-align:left;border-bottom:1px solid #ddd}.passed{color:#137333}.failed{color:#c5221f}.skipped{color:#b06000}</style>"
  echo "<h1>Release check</h1><p>$(echo "$BRANCH @ $SHA$DIRTY · $(date '+%Y-%m-%d %H:%M')" | html)</p>"
  echo "<table><tr><th>Check</th><th>Status</th><th>Detail</th><th>Time</th><th>Log</th></tr>"
  i=0
  for k in $KEYS; do
    echo "<tr><td>${NAMES[$i]}</td><td class=${STATUS[$i]}>${STATUS[$i]}</td><td>$(echo "${DETAIL[$i]}" | html)</td><td>$(fmt_time "${SECS[$i]}")</td><td><a href=\"$k.log\">$k.log</a></td></tr>"
    i=$((i + 1))
  done
  echo "</table><p><b>Result: $(echo "$RESULT" | html)</b></p>"
  [ -n "$CHANGES" ] && echo "<pre>$(printf '%s' "$CHANGES" | html)</pre>"
} >"$REPORT"

echo
echo "${B}Release check${N}"
for i in 0 1 2 3 4; do printf '%s %-14s %s\n' "$(mark "${STATUS[$i]}")" "${NAMES[$i]}" "${DETAIL[$i]}"; done
[ -n "$CHANGES" ] && { echo; printf '%s' "$CHANGES"; }
echo
if [ "$READY" -eq 1 ]; then echo "${B}Result: ${G}$RESULT${N}"; else echo "${B}Result: ${R}$RESULT${N}"; fi
echo "Report: ${REPORT#$ROOT/}"
[ "$READY" -eq 1 ]
