#!/usr/bin/env bash
# Foreman dev check: the whole pre-commit cycle in one command.
#
#   cargo check-all [--printers] [--keep]
#   bash scripts/dev-check.sh [--printers] [--keep]
#
#   --printers  also run read-only live checks against HellV0 (A1 on hold)
#               (status, capabilities incl. nozzle, snapshot). Never sends commands.
#   --keep      leave the freshly built server running afterwards.
#
# stdout: one line per step — STEP  PASS/FAIL  duration  [first error line].
# Full output of every step: .dev-logs/<step>.log (gitignored).
# Exit status: non-zero if any step failed.
#
# In CI (CI=true) foreman-desktop is excluded from clippy/test, like ci.yml does.

set -u

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT" || exit 2

PRINTERS=0
KEEP=0
for arg in "$@"; do
  case "$arg" in
    --printers) PRINTERS=1 ;;
    --keep) KEEP=1 ;;
    -h | --help)
      sed -n '2,15p' "$0"
      exit 0
      ;;
    *)
      echo "dev-check: unknown option '$arg' (use --printers, --keep)" >&2
      exit 2
      ;;
  esac
done

LOGS="$ROOT/.dev-logs"
mkdir -p "$LOGS"
rm -f "$LOGS"/*.log

PORT="${FOREMAN__PORT:-9090}"
BASE="http://127.0.0.1:$PORT"
case "$(uname -s)" in
  MINGW* | MSYS* | CYGWIN*) EXE=".exe" ;;
  *) EXE="" ;;
esac
BIN="$ROOT/target/debug/foreman-server$EXE"

WS_FLAGS=()
if [ "${CI:-}" = "true" ]; then
  WS_FLAGS=(--workspace --exclude foreman-desktop)
fi

FAILED=0
SERVER_PID=""
TOKEN=""

now_ms() {
  local n
  n="$(date +%s%N 2>/dev/null)"
  case "$n" in
    *N | "") echo $(($(date +%s) * 1000)) ;;
    *) echo $((n / 1000000)) ;;
  esac
}

# First line of a log that looks like an error; falls back to the last non-empty line.
first_error() {
  local log="$1" line
  line="$(grep -m1 -E '^error(\[|:)|^Diff in |^thread .* panicked|FAILED|^failures:|error TS[0-9]+|^ *(FAIL|×) |ERR!|^FAIL:' "$log" 2>/dev/null)"
  if [ -z "$line" ]; then
    line="$(grep -v '^[[:space:]]*$' "$log" 2>/dev/null | tail -n 1)"
  fi
  # Strip ANSI colour codes and squeeze whitespace.
  printf '%s' "$line" | sed -e 's/\x1b\[[0-9;]*m//g' -e 's/[[:space:]]\+/ /g' | cut -c1-160
}

report() { # name status ms [detail]
  local name="$1" status="$2" ms="$3" detail="${4:-}"
  local secs
  secs="$(awk -v m="$ms" 'BEGIN { printf "%.1fs", m / 1000 }')"
  if [ -n "$detail" ]; then
    printf '%-18s %-4s %7s  %s\n' "$name" "$status" "$secs" "$detail"
  else
    printf '%-18s %-4s %7s\n' "$name" "$status" "$secs"
  fi
}

# step NAME CMD...  — run CMD with all output to .dev-logs/NAME.log.
step() {
  local name="$1"
  shift
  local log="$LOGS/$name.log" start rc
  start="$(now_ms)"
  "$@" >"$log" 2>&1
  rc=$?
  local ms=$(($(now_ms) - start))
  if [ $rc -eq 0 ]; then
    report "$name" PASS "$ms"
  else
    FAILED=1
    report "$name" FAIL "$ms" "$(first_error "$log")"
  fi
  return $rc
}

skip() { # name reason
  printf '%-18s %-4s %7s  %s\n' "$1" SKIP "-" "$2"
}

# --- server lifecycle -------------------------------------------------------

# PIDs listening on $PORT.
port_pids() {
  if command -v ss >/dev/null 2>&1; then
    ss -ltnpH "sport = :$PORT" 2>/dev/null | grep -o 'pid=[0-9]*' | cut -d= -f2 | sort -u
  elif command -v lsof >/dev/null 2>&1; then
    lsof -t -iTCP:"$PORT" -sTCP:LISTEN 2>/dev/null | sort -u
  elif command -v netstat >/dev/null 2>&1; then
    netstat -ano 2>/dev/null | awk -v p=":$PORT" '$2 ~ p"$" && $4 == "LISTENING" { print $5 }' | sort -u
  fi
}

# Executable path of a PID ("" if unknown). A rebuilt binary shows "(deleted)".
pid_exe() {
  local p="$1" exe=""
  if [ -e "/proc/$p/exe" ]; then
    exe="$(readlink "/proc/$p/exe" 2>/dev/null)"
    exe="${exe% (deleted)}"
  elif command -v wmic >/dev/null 2>&1; then
    exe="$(wmic process where "ProcessId=$p" get ExecutablePath 2>/dev/null | sed -n 2p | tr -d '\r ')"
  fi
  printf '%s' "$exe"
}

same_binary() { # exe
  local exe="$1"
  [ -n "$exe" ] || return 1
  [ "$exe" = "$BIN" ] && return 0
  # Windows paths from wmic: compare case-insensitively with forward slashes.
  if [ -n "$EXE" ] && command -v cygpath >/dev/null 2>&1; then
    [ "$(cygpath -u "$exe" | tr 'A-Z' 'a-z')" = "$(printf '%s' "$BIN" | tr 'A-Z' 'a-z')" ] && return 0
  fi
  return 1
}

kill_pid() {
  local p="$1"
  if [ -n "$EXE" ] && command -v taskkill >/dev/null 2>&1; then
    taskkill //PID "$p" //F >/dev/null 2>&1
  else
    kill "$p" 2>/dev/null
  fi
}

# Stop Foreman servers: anything on $PORT that is our binary, plus any process whose
# executable is exactly our binary. Never a name pattern. Something else on the
# port is an error, not a kill.
stop_server() {
  local p exe found=0 foreign=""
  local pids
  pids="$(port_pids)"
  if [ -d /proc ]; then
    for d in /proc/[0-9]*; do
      exe="$(readlink "$d/exe" 2>/dev/null)" || continue
      exe="${exe% (deleted)}"
      [ "$exe" = "$BIN" ] && pids="$pids ${d#/proc/}"
    done
  fi
  for p in $(printf '%s\n' $pids | sort -u); do
    exe="$(pid_exe "$p")"
    if same_binary "$exe"; then
      echo "stopping pid $p ($exe)"
      kill_pid "$p"
      found=1
    else
      foreign="$foreign $p(${exe:-unknown})"
    fi
  done
  # Wait up to 10 s for the port to free.
  for _ in $(seq 1 50); do
    [ -z "$(port_pids)" ] && break
    sleep 0.2
  done
  if [ -n "$foreign" ]; then
    echo "error: port $PORT is held by a process that is not $BIN:$foreign"
    return 1
  fi
  if [ -n "$(port_pids)" ]; then
    echo "error: port $PORT still in use after stopping"
    return 1
  fi
  [ $found -eq 1 ] && echo "stopped" || echo "nothing running"
  return 0
}

health_field() { # json field
  printf '%s' "$1" | sed -n "s/.*\"$2\":\"\([^\"]*\)\".*/\1/p"
}

start_server() {
  [ -x "$BIN" ] || { echo "error: $BIN not built"; return 1; }
  "$BIN" >"$LOGS/server.log" 2>&1 &
  SERVER_PID=$!
  echo "started pid $SERVER_PID, log .dev-logs/server.log"
  local body=""
  for _ in $(seq 1 120); do
    if ! kill -0 "$SERVER_PID" 2>/dev/null; then
      echo "error: server exited during startup (see .dev-logs/server.log)"
      grep -m1 -iE 'error|panicked' "$LOGS/server.log"
      SERVER_PID=""
      return 1
    fi
    body="$(curl -sf --max-time 2 "$BASE/api/health" 2>/dev/null)" && break
    sleep 0.5
  done
  [ -n "$body" ] || { echo "error: /api/health did not answer within 60 s"; return 1; }
  echo "health: $body"
  printf '%s' "$body" >"$LOGS/.health.json"
}

check_commit() {
  local body head commit built
  body="$(cat "$LOGS/.health.json" 2>/dev/null)"
  head="$(git rev-parse HEAD)"
  commit="$(health_field "$body" commit)"
  built="$(health_field "$body" built_at)"
  echo "HEAD    $head"
  echo "running $commit (built_at $built)"
  if ! sed -e 's/\x1b\[[0-9;]*m//g' "$LOGS/server.log" | grep -qE "commit=\"?$head"; then
    echo "error: startup log does not show commit=$head"
    return 1
  fi
  if [ "$commit" != "$head" ]; then
    echo "error: running commit ${commit:-none} != HEAD $head"
    return 1
  fi
}

cleanup() {
  if [ -n "$SERVER_PID" ] && [ $KEEP -eq 0 ]; then
    kill_pid "$SERVER_PID"
    wait "$SERVER_PID" 2>/dev/null
    SERVER_PID=""
  fi
}
trap 'cleanup; exit 130' INT TERM

# --- printers (read-only) ---------------------------------------------------

api_get() { # path → body on stdout; fails on non-2xx
  curl -sf --max-time 15 -H "Authorization: Bearer $TOKEN" "$BASE$1"
}

get_token() {
  command -v jq >/dev/null 2>&1 || { echo "error: jq is required for --printers"; return 1; }
  TOKEN="$("$BIN" dev-token)" || { echo "error: dev-token failed"; return 1; }
  echo "dev-token ok (${TOKEN:0:8}…)"
  api_get /api/printers >"$LOGS/.printers.json" || { echo "error: GET /api/printers failed"; return 1; }
  jq -c '.[] | {id, name, firmware}' "$LOGS/.printers.json"
}

printer_id() { # jq select filter → first matching printer id
  jq -r "[.[] | select($1) | .id][0] // empty" "$LOGS/.printers.json" 2>/dev/null
}

check_status() { # id
  local body
  body="$(api_get "/api/printers/$1/status")" || { echo "error: GET status failed"; return 1; }
  echo "$body"
  printf '%s' "$body" | jq -e '.state | type == "string"' >/dev/null ||
    { echo "error: status has no state"; return 1; }
  echo "state: $(printf '%s' "$body" | jq -r .state)"
}

check_caps() { # id
  local body
  body="$(api_get "/api/printers/$1/capabilities")" || { echo "error: GET capabilities failed"; return 1; }
  echo "$body"
  printf '%s' "$body" | jq -e '.nozzle_diameter | type == "number" and . > 0' >/dev/null ||
    { echo "error: capabilities have no nozzle_diameter"; return 1; }
  echo "nozzle_diameter: $(printf '%s' "$body" | jq -r .nozzle_diameter)"
}

check_snapshot() { # id
  local out="$LOGS/.snapshot-$1" meta code type size
  meta="$(curl -s --max-time 20 -o "$out" -w '%{http_code} %{content_type} %{size_download}' \
    -H "Authorization: Bearer $TOKEN" "$BASE/api/printers/$1/snapshot")"
  read -r code type size <<<"$meta"
  echo "HTTP $code, $type, $size bytes"
  [ "$code" = "200" ] || { echo "error: snapshot HTTP $code: $(head -c 200 "$out")"; return 1; }
  case "$type" in image/*) ;; *) echo "error: snapshot is $type, not an image"; return 1 ;; esac
  [ "${size:-0}" -gt 0 ] || { echo "error: empty snapshot"; return 1; }
}

printer_checks() { # label id
  local label="$1" id="$2"
  if [ -z "$id" ]; then
    skip "$label" "printer not configured in this server"
    FAILED=1
    return
  fi
  step "$label-status" check_status "$id"
  step "$label-caps" check_caps "$id"
  step "$label-snapshot" check_snapshot "$id"
}

# Run a command inside frontend/ (subshell, so the script's cwd is untouched).
fe() (
  cd "$ROOT/frontend" && "$@"
)

# --- run ---------------------------------------------------------------------

printf '%-18s %-4s %7s  %s\n' STEP RESULT TIME DETAIL

step stop-server stop_server
step fmt cargo fmt --all -- --check
step clippy cargo clippy "${WS_FLAGS[@]}" --all-targets -- -D warnings
step test cargo test --all "${WS_FLAGS[@]}"

if [ ! -d frontend/node_modules ]; then
  step frontend-deps fe npm ci
fi
step frontend-types fe npx --no-install tsc -p tsconfig.app.json --noEmit
step frontend-build fe npx --no-install vite build
step frontend-test fe npm test

if step server-build cargo build -p foreman-server; then
  if step server-start start_server; then
    step server-commit check_commit
    if [ $PRINTERS -eq 1 ]; then
      if step printers-auth get_token; then
        printer_checks hellv0 "$(printer_id '.name == "HellV0"')"
        # A1 (Bambu) checks are on hold (owner, 2026-11-11); re-add with:
        #   printer_checks a1 "$(printer_id '.firmware == "bambu"')"
      fi
    fi
  fi
else
  skip server-start "server-build failed"
fi

if [ -n "$SERVER_PID" ]; then
  if [ $KEEP -eq 1 ]; then
    echo "server left running: pid $SERVER_PID on $BASE (log .dev-logs/server.log)"
  else
    cleanup
  fi
fi

if [ $FAILED -ne 0 ]; then
  echo "dev-check: FAILED — read .dev-logs/<step>.log for the failed steps"
  exit 1
fi
echo "dev-check: all passed"
