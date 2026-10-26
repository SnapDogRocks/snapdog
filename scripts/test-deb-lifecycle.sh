#!/bin/sh
# Lifecycle test for the snapdog and snapdog-client .deb maintainer scripts.
#
# Installs, upgrades, removes and purges the packages in a Debian 12 container that
# runs systemd as PID 1, and checks the state of both services after each step.
# Every scenario starts in a fresh container.
#
#   scripts/test-deb-lifecycle.sh DEB_DIR [OLD_DEB_DIR]
#
# DEB_DIR holds snapdog_*.deb and snapdog-client_*.deb for the architecture of the
# Docker host (a stub binary in place of the real one is enough, see below). With
# OLD_DEB_DIR, which holds the same files from a release without maintainer scripts
# (0.28.2 and older), the upgrade from it is tested as well.
#
# SNAPDOG_LIFECYCLE_SCENARIOS="name name" runs only those scenarios (see the list below).
#
# Needs a Docker that allows privileged containers with a cgroup namespace of the host
# (Docker Engine, colima). It does not run in CI: the release workflow only smoke-tests
# a direct install without systemd.
#
# A stub binary that sleeps is enough to package, for example:
#   printf '#!/bin/sh\nexec sleep 100000\n' >stub && install -Dm755 stub \
#     target/<triple>/release/snapdog   (and the same for snapdog-client)
#   cargo deb -p snapdog --target <triple> --no-build --no-strip
set -eu

IMAGE=snapdog-deb-lifecycle-test

if [ "${1:-}" != --inside ]; then
    [ $# -ge 1 ] || { sed -n '2,14p' "$0" >&2; exit 2; }
    deb_dir=$1
    old_dir=${2:-}
    docker build -q -t "$IMAGE" - <<'EOF' >/dev/null
FROM debian:12
# The image ships a policy-rc.d that forbids starting services.
RUN apt-get update \
 && apt-get install -y --no-install-recommends systemd systemd-sysv dbus libasound2 \
      libavahi-compat-libdnssd1 libssl3 \
 && rm -f /usr/sbin/policy-rc.d && apt-get clean
CMD ["/lib/systemd/systemd"]
EOF
    scenarios=${SNAPDOG_LIFECYCLE_SCENARIOS:-$(sh "$0" --inside --list)}
    failed=0
    for scenario in $scenarios; do
        case $scenario in upgrade_from_old_*) [ -n "$old_dir" ] || continue ;; esac
        name=snapdog-deb-lifecycle-$$
        trap 'docker rm -f "$name" >/dev/null 2>&1 || true' EXIT
        docker run -d --name "$name" --privileged --cgroupns=host \
            -v /sys/fs/cgroup:/sys/fs/cgroup:rw --tmpfs /run --tmpfs /run/lock \
            "$IMAGE" >/dev/null
        docker exec "$name" mkdir /deb /deb-old
        docker cp "$deb_dir/." "$name:/deb"
        [ -z "$old_dir" ] || docker cp "$old_dir/." "$name:/deb-old"
        docker cp "$0" "$name:/lifecycle.sh"
        i=0
        until docker exec "$name" systemctl is-system-running 2>/dev/null \
            | grep -q '^\(running\|degraded\)$'; do
            i=$((i + 1))
            [ "$i" -lt 30 ] || { echo "systemd did not come up" >&2; exit 1; }
            sleep 1
        done
        if docker exec "$name" sh /lifecycle.sh --inside "$scenario"; then :; else
            failed=$((failed + 1))
        fi
        docker rm -f "$name" >/dev/null
    done
    if [ "$failed" -ne 0 ]; then
        echo "$failed scenario(s) failed" >&2
        exit 1
    fi
    echo "all scenarios passed"
    exit 0
fi

# ---- inside the container ----------------------------------------------------------

scenario_list="fresh_install
upgrade_restarts_running_services
upgrade_keeps_a_stopped_server_stopped
upgrade_respects_a_disabled_client
reinstall_after_remove_restores_a_running_server
reinstall_after_remove_leaves_a_stopped_server_alone
reinstall_after_remove_leaves_a_disabled_server_alone
purge_removes_state
purge_while_running
purge_after_remove
upgrade_from_old_release
upgrade_from_old_release_with_enabled_server
upgrade_from_old_release_with_masked_client"

if [ "${2:-}" = --list ]; then
    echo "$scenario_list"
    exit 0
fi

export DEBIAN_FRONTEND=noninteractive
arch=$(dpkg --print-architecture)
new_server=$(ls /deb/snapdog_*_"$arch".deb)
new_client=$(ls /deb/snapdog-client_*_"$arch".deb)
old_server=$(ls /deb-old/snapdog_*_"$arch".deb 2>/dev/null || true)
old_client=$(ls /deb-old/snapdog-client_*_"$arch".deb 2>/dev/null || true)
failures=0
step_name=setup

# state UNIT ENABLED ACTIVE: expect the unit file state and the activity to match.
state() {
    got_enabled=$(systemctl is-enabled "$1" 2>&1 || true)
    got_active=$(systemctl is-active "$1" 2>&1 || true)
    if [ "$got_enabled" = "$2" ] && [ "$got_active" = "$3" ]; then
        echo "  ok    $1 $2/$3"
    else
        echo "  FAIL  $1: expected $2/$3, got $got_enabled/$got_active ($step_name)"
        failures=$((failures + 1))
    fi
}
# gone UNIT: expect the unit to be unknown to systemd, as after a purge.
gone() {
    if systemctl cat "$1" >/dev/null 2>&1; then
        echo "  FAIL  $1 is still known to systemd ($step_name)"
        failures=$((failures + 1))
    else
        echo "  ok    $1 is unknown to systemd"
    fi
}
pid() { systemctl show -p MainPID --value "$1"; }
# absent PATH...: expect none of the paths to exist (a dangling link counts).
absent() {
    for p in "$@"; do
        if [ -e "$p" ] || [ -L "$p" ]; then
            echo "  FAIL  $p exists ($step_name)"
            failures=$((failures + 1))
        else
            echo "  ok    $p is gone"
        fi
    done
}
present() {
    if [ -e "$1" ] || [ -L "$1" ]; then echo "  ok    $1 exists"; else
        echo "  FAIL  $1 is missing ($step_name)"
        failures=$((failures + 1))
    fi
}
changed_pid() { # UNIT OLDPID
    new=$(pid "$1")
    if [ "$new" != 0 ] && [ "$new" != "$2" ]; then echo "  ok    $1 restarted ($2 -> $new)"; else
        echo "  FAIL  $1 was not restarted (pid $2 -> $new) ($step_name)"
        failures=$((failures + 1))
    fi
}
same_pid() {
    new=$(pid "$1")
    if [ "$new" = "$2" ]; then echo "  ok    $1 left alone (pid $new)"; else
        echo "  FAIL  $1 pid changed $2 -> $new ($step_name)"
        failures=$((failures + 1))
    fi
}
step() { step_name=$*; echo "-- $*"; }
install_() { apt-get install -y -qq "$@" >/tmp/apt.log 2>&1 || { cat /tmp/apt.log; exit 1; }; sleep 1; }
reinstall() { apt-get install -y -qq --reinstall "$@" >/tmp/apt.log 2>&1 || { cat /tmp/apt.log; exit 1; }; sleep 1; }
remove() { apt-get remove -y -qq "$@" >/tmp/apt.log 2>&1 || { cat /tmp/apt.log; exit 1; }; }
purge() { apt-get purge -y -qq "$@" >/tmp/apt.log 2>&1 || { cat /tmp/apt.log; exit 1; }; }
start_server() { systemctl enable --now snapdog >/dev/null 2>&1; sleep 1; }

scenario_fresh_install() {
    step "fresh install of both"
    install_ "$new_server" "$new_client"
    state snapdog disabled inactive
    state snapdog-client enabled active
    present /etc/systemd/system/multi-user.target.wants/snapdog-client.service
    absent /etc/systemd/system/multi-user.target.wants/snapdog.service
}

scenario_upgrade_restarts_running_services() {
    install_ "$new_server" "$new_client"
    start_server
    state snapdog enabled active
    s=$(pid snapdog) c=$(pid snapdog-client)
    step "upgrade (reinstall of the same version) with both running"
    reinstall "$new_server" "$new_client"
    state snapdog enabled active
    state snapdog-client enabled active
    changed_pid snapdog "$s"
    changed_pid snapdog-client "$c"
}

scenario_upgrade_keeps_a_stopped_server_stopped() {
    install_ "$new_server" "$new_client"
    systemctl enable snapdog >/dev/null 2>&1
    state snapdog enabled inactive
    step "upgrade with the server enabled but stopped"
    reinstall "$new_server" "$new_client"
    state snapdog enabled inactive
}

scenario_upgrade_respects_a_disabled_client() {
    install_ "$new_server" "$new_client"
    systemctl disable --now snapdog-client >/dev/null 2>&1
    state snapdog-client disabled inactive
    step "upgrade with the client disabled by the administrator"
    reinstall "$new_server" "$new_client"
    state snapdog-client disabled inactive
}

scenario_reinstall_after_remove_restores_a_running_server() {
    install_ "$new_server" "$new_client"
    start_server
    state snapdog enabled active
    step "remove both"
    remove snapdog snapdog-client
    state snapdog masked inactive
    state snapdog-client masked inactive
    step "install both again"
    install_ "$new_server" "$new_client"
    state snapdog enabled active
    state snapdog-client enabled active
    absent /run/snapdog.was-active
}

scenario_reinstall_after_remove_leaves_a_stopped_server_alone() {
    install_ "$new_server" "$new_client"
    systemctl enable snapdog >/dev/null 2>&1
    step "remove and install again, the server enabled but stopped throughout"
    remove snapdog snapdog-client
    install_ "$new_server" "$new_client"
    state snapdog enabled inactive
    absent /run/snapdog.was-active
}

scenario_reinstall_after_remove_leaves_a_disabled_server_alone() {
    install_ "$new_server" "$new_client"
    systemctl start snapdog
    state snapdog disabled active
    step "remove while the server runs without being enabled, then install again"
    remove snapdog snapdog-client
    install_ "$new_server" "$new_client"
    state snapdog disabled inactive
    absent /run/snapdog.was-active
}

scenario_purge_removes_state() {
    install_ "$new_server" "$new_client"
    start_server
    echo state >/var/lib/snapdog/pairing.json
    present /var/lib/private/snapdog/pairing.json
    present /var/lib/snapdog
    step "purge both"
    purge snapdog snapdog-client
    gone snapdog.service
    gone snapdog-client.service
    absent /var/lib/private/snapdog /var/lib/snapdog /etc/snapdog /etc/default/snapdog \
        /etc/default/snapdog-client /run/snapdog.was-active \
        /var/lib/systemd/deb-systemd-helper-enabled/multi-user.target.wants/snapdog.service \
        /var/lib/systemd/deb-systemd-helper-masked/snapdog.service \
        /var/lib/systemd/deb-systemd-helper-masked/snapdog-client.service
}

scenario_purge_while_running() {
    install_ "$new_server" "$new_client"
    start_server
    step "purge both while they run"
    purge snapdog snapdog-client
    gone snapdog.service
    gone snapdog-client.service
    absent /var/lib/private/snapdog /var/lib/snapdog /run/snapdog.was-active
}

scenario_purge_after_remove() {
    install_ "$new_server" "$new_client"
    start_server
    remove snapdog snapdog-client
    present /run/snapdog.was-active
    step "purge after remove"
    purge snapdog snapdog-client
    absent /var/lib/private/snapdog /var/lib/snapdog /run/snapdog.was-active
}

scenario_upgrade_from_old_release() {
    step "install the old release (no maintainer scripts)"
    install_ "$old_server" "$old_client"
    state snapdog disabled inactive
    state snapdog-client disabled inactive
    step "first upgrade to the release with scripts: the client is enabled and started"
    install_ "$new_server" "$new_client"
    state snapdog disabled inactive
    state snapdog-client enabled active
    step "later upgrades respect what the administrator does"
    systemctl disable --now snapdog-client >/dev/null 2>&1
    reinstall "$new_server" "$new_client"
    state snapdog-client disabled inactive
}

scenario_upgrade_from_old_release_with_enabled_server() {
    install_ "$old_server" "$old_client"
    start_server
    state snapdog enabled active
    s=$(pid snapdog)
    step "first upgrade with the server enabled and running by hand"
    install_ "$new_server" "$new_client"
    state snapdog enabled active
    changed_pid snapdog "$s"
    state snapdog-client enabled active
}

scenario_upgrade_from_old_release_with_masked_client() {
    install_ "$old_server" "$old_client"
    systemctl mask snapdog-client >/dev/null 2>&1
    state snapdog-client masked inactive
    step "first upgrade with the client masked beforehand"
    install_ "$new_server" "$new_client"
    state snapdog-client masked inactive
    state snapdog disabled inactive
}

echo "== ${2}"
"scenario_${2}"
exit "$((failures > 0))"
