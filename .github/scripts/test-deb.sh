#!/usr/bin/env bash
# Install a foreman-server .deb into a Debian 12 container that runs systemd as
# PID 1, then check the service: running as user foreman, data in /var/lib/foreman,
# listening on all interfaces, /api/health and the web UI answering. Then purge.
# CI (release.yml, job deb-test) and local use:
#   .github/scripts/test-deb.sh path/to/foreman-server_X_amd64.deb
set -euo pipefail

deb=$(realpath "$1")
image=foreman-deb-test:bookworm
name=foreman-deb-test-$$

docker build -q -t "$image" - >/dev/null <<'EOF'
FROM debian:bookworm-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends systemd systemd-sysv curl ca-certificates procps \
 && rm -rf /var/lib/apt/lists/*
# Docker's Debian images ship a policy-rc.d that stops packages from starting
# services; a real host has none, and the test is whether the .deb starts it.
RUN rm -f /usr/sbin/policy-rc.d
STOPSIGNAL SIGRTMIN+3
CMD ["/lib/systemd/systemd"]
EOF

cleanup() {
  status=$?
  if [[ $status != 0 ]]; then
    echo "== FAILED (exit $status); service log:" >&2
    docker exec "$name" journalctl -u foreman --no-pager -n 40 >&2 || true
  fi
  docker rm -f "$name" >/dev/null 2>&1 || true
}
trap cleanup EXIT

docker run -d --name "$name" --privileged --cgroupns=host \
  -v /sys/fs/cgroup:/sys/fs/cgroup:rw --tmpfs /run --tmpfs /run/lock \
  -v "$deb:/tmp/foreman-server.deb:ro" "$image" >/dev/null

ctr() { docker exec "$name" "$@"; }

# Wait for systemd to finish booting.
for _ in $(seq 1 30); do
  state=$(ctr systemctl is-system-running 2>/dev/null || true)
  [[ $state == running || $state == degraded ]] && break
  sleep 1
done
echo "systemd: $state"

echo "== install"
ctr apt-get install -y /tmp/foreman-server.deb 2>&1 | grep -E "^(Setting up|Unpacking|Created symlink)|error" || true
ctr dpkg -s foreman-server | grep -E "^(Version|Depends):"

echo "== service"
for _ in $(seq 1 30); do
  code=$(ctr curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:9090/api/health || true)
  [[ $code == 200 ]] && break
  sleep 1
done
ctr systemctl is-enabled foreman
ctr systemctl is-active foreman
ctr curl -fsS http://127.0.0.1:9090/api/health; echo
user=$(ctr ps -o user= -C foreman-server | tr -d ' ')
echo "process user: $user"
[[ $user == foreman ]]
ctr stat -c '%U:%G %a %n' /var/lib/foreman
[[ $(ctr stat -c '%U' /var/lib/foreman) == foreman ]]
ctr test -f /var/lib/foreman/foreman.db && echo "database: /var/lib/foreman/foreman.db"
ctr systemctl show foreman -p Environment --value | tr ' ' '\n' | grep FOREMAN__HOST=0.0.0.0
# Port 9090 = 0x2382; local address all zeros = 0.0.0.0 / [::].
ctr cat /proc/net/tcp /proc/net/tcp6 | grep -q '00000000:2382 ' && echo "listening on all interfaces on 9090"
ctr curl -fsS http://127.0.0.1:9090/ | grep -o '<title>[^<]*</title>'
ctr sh -c 'journalctl -u foreman --no-pager | grep -m1 "foreman-server starting"'

echo "== restart survives"
ctr systemctl restart foreman
sleep 2
ctr systemctl is-active foreman

echo "== purge"
ctr apt-get purge -y foreman-server >/dev/null 2>&1
if ctr test -e /var/lib/foreman || ctr getent passwd foreman >/dev/null; then
  echo "purge left /var/lib/foreman or the foreman user behind" >&2
  exit 1
fi
echo "purged: data dir and user removed"
echo "DEB TEST PASSED"
