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
#
# An "upgrade" here is a reinstall of the same version: dpkg runs the same scripts with
# the same arguments. Scenarios named reboot_* restart the container between two phases,
# which empties /run like a reboot does and keeps everything else.
# The scenario_* functions are called by name, which shellcheck cannot see.
# shellcheck disable=SC2329
set -eu

IMAGE=snapdog-deb-lifecycle-test

wait_for_systemd() { # CONTAINER
    i=0
    until docker exec "$1" systemctl is-system-running 2>/dev/null \
        | grep -q '^\(running\|degraded\)$'; do
        i=$((i + 1))
        [ "$i" -lt 60 ] || { echo "systemd did not come up" >&2; return 1; }
        sleep 0.5
    done
}

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
        case $scenario in upgrade_from_old_* | install_after_removing_old_*) [ -n "$old_dir" ] || continue ;; esac
        name=snapdog-deb-lifecycle-$$
        trap 'docker rm -f "$name" >/dev/null 2>&1 || true' EXIT
        docker run -d --name "$name" --privileged --cgroupns=host \
            -v /sys/fs/cgroup:/sys/fs/cgroup:rw --tmpfs /run --tmpfs /run/lock \
            "$IMAGE" >/dev/null
        docker exec "$name" mkdir /deb /deb-old
        docker cp "$deb_dir/." "$name:/deb"
        [ -z "$old_dir" ] || docker cp "$old_dir/." "$name:/deb-old"
        docker cp "$0" "$name:/lifecycle.sh"
        wait_for_systemd "$name"
        ok=1
        case $scenario in
            reboot_*)
                docker exec "$name" sh /lifecycle.sh --inside "$scenario" 1 || ok=0
                if [ "$ok" = 1 ]; then
                    docker restart "$name" >/dev/null
                    wait_for_systemd "$name"
                    docker exec "$name" sh /lifecycle.sh --inside "$scenario" 2 || ok=0
                fi
                ;;
            *) docker exec "$name" sh /lifecycle.sh --inside "$scenario" 0 || ok=0 ;;
        esac
        [ "$ok" = 1 ] || failed=$((failed + 1))
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
upgrade_starts_a_stopped_enabled_client
upgrade_respects_a_disabled_client
two_upgrades_in_a_row
masked_server_stays_masked
reinstall_after_remove_restores_a_running_server
reboot_reinstall_after_remove_restores_a_running_server
reinstall_after_remove_leaves_a_stopped_server_alone
reinstall_after_remove_leaves_a_disabled_server_alone
interrupted_remove_leaves_nothing_for_a_later_upgrade
policy_rc_d_101_keeps_state_of_a_running_server
failed_server_unit
purge_removes_state
purge_while_running
purge_after_remove
upgrade_from_old_release
upgrade_from_old_release_with_client_enabled_by_hand
upgrade_from_old_release_with_client_disabled_again
upgrade_from_old_release_with_enabled_server
upgrade_from_old_release_with_masked_client
upgrade_from_old_release_with_masked_server
install_after_removing_old_release"

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
state_dir=/var/lib/snapdog
note=$state_dir/.dpkg-was-running

# Wait for what the scripts started to settle: a check passes at once if it holds, and is
# retried for up to 10 s if not, so that no check depends on a fixed pause.
retry() {
    i=0
    until "$@"; do
        i=$((i + 1))
        [ "$i" -lt 20 ] || return 1
        sleep 0.5
    done
}

is_inactive() { [ "$(systemctl is-active "$1" 2>&1 || true)" = inactive ]; }
is_state() { # UNIT ENABLED ACTIVE
    [ "$(systemctl is-enabled "$1" 2>&1 || true)" = "$2" ] \
        && [ "$(systemctl is-active "$1" 2>&1 || true)" = "$3" ]
}
# state UNIT ENABLED ACTIVE: expect the unit file state and the activity to match.
state() {
    if retry is_state "$1" "$2" "$3"; then
        echo "  ok    $1 $2/$3"
    else
        echo "  FAIL  $1: expected $2/$3, got $(systemctl is-enabled "$1" 2>&1 || true)/$(systemctl is-active "$1" 2>&1 || true) ($step_name)"
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
is_restarted() { new=$(pid "$1"); [ "$new" != 0 ] && [ "$new" != "$2" ]; }
changed_pid() { # UNIT OLDPID
    if retry is_restarted "$1" "$2"; then echo "  ok    $1 restarted ($2 -> $(pid "$1"))"; else
        echo "  FAIL  $1 was not restarted (pid $2 -> $(pid "$1")) ($step_name)"
        failures=$((failures + 1))
    fi
}
same_pid() { # UNIT PID
    if [ "$(pid "$1")" = "$2" ]; then echo "  ok    $1 left alone (pid $2)"; else
        echo "  FAIL  $1 pid changed $2 -> $(pid "$1") ($step_name)"
        failures=$((failures + 1))
    fi
}
step() { step_name=$*; echo "-- $*"; }
apt_quiet() { apt-get "$@" >/tmp/apt.log 2>&1 || { cat /tmp/apt.log; return 1; }; }
install_() { apt_quiet install -y -qq "$@" || exit 1; }
reinstall() { apt_quiet install -y -qq --reinstall "$@" || exit 1; }
remove() { apt_quiet remove -y -qq "$@" || exit 1; }
purge() { apt_quiet purge -y -qq "$@" || exit 1; }
# A Type=simple unit counts as started once it is forked, before systemd has made its
# state directory, so wait for the directory too.
has_state_dir() { [ -d /var/lib/private/snapdog ]; }
start_server() {
    systemctl enable --now snapdog >/dev/null 2>&1
    retry has_state_dir || { echo "  FAIL  no state directory after the start ($step_name)"; failures=$((failures + 1)); }
}
# A copy of a .deb whose postrm fails when `remove` is called and /tmp/failrm exists.
build_failing_postrm_deb() { # DEB OUT
    rm -rf /tmp/v
    mkdir /tmp/v
    dpkg-deb -R "$1" /tmp/v
    # shellcheck disable=SC2016  # $1 is for the postrm that is patched, not for this shell
    sed -i '0,/^set -e$/s//set -e\n[ ! -e \/tmp\/failrm ] || [ "$1" != remove ] || { echo "postrm: forced failure" >\&2; exit 1; }/' /tmp/v/DEBIAN/postrm
    dpkg-deb -b /tmp/v "$2" >/dev/null
}

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
    step "upgrade with both running"
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

scenario_upgrade_starts_a_stopped_enabled_client() {
    install_ "$new_server" "$new_client"
    systemctl stop snapdog-client
    state snapdog-client enabled inactive
    step "upgrade with the client enabled but stopped: it is started again (start = true)"
    reinstall "$new_server" "$new_client"
    state snapdog-client enabled active
}

scenario_upgrade_respects_a_disabled_client() {
    install_ "$new_server" "$new_client"
    systemctl disable --now snapdog-client >/dev/null 2>&1
    state snapdog-client disabled inactive
    step "upgrade with the client disabled by the administrator"
    reinstall "$new_server" "$new_client"
    state snapdog-client disabled inactive
}

scenario_two_upgrades_in_a_row() {
    install_ "$new_server" "$new_client"
    start_server
    s=$(pid snapdog)
    step "first upgrade"
    reinstall "$new_server" "$new_client"
    state snapdog enabled active
    changed_pid snapdog "$s"
    s=$(pid snapdog)
    step "second upgrade"
    reinstall "$new_server" "$new_client"
    state snapdog enabled active
    state snapdog-client enabled active
    changed_pid snapdog "$s"
    step "the administrator disables the server without stopping it, then a third upgrade"
    systemctl disable snapdog >/dev/null 2>&1
    reinstall "$new_server" "$new_client"
    state snapdog disabled active
    step "the administrator enables it again and stops it, then a fourth upgrade"
    systemctl enable snapdog >/dev/null 2>&1
    systemctl stop snapdog
    reinstall "$new_server" "$new_client"
    state snapdog enabled inactive
}

scenario_masked_server_stays_masked() {
    install_ "$new_server" "$new_client"
    systemctl mask --now snapdog snapdog-client >/dev/null 2>&1
    state snapdog masked inactive
    state snapdog-client masked inactive
    step "upgrade with both masked by the administrator"
    reinstall "$new_server" "$new_client"
    state snapdog masked inactive
    state snapdog-client masked inactive
    step "remove and install again"
    remove snapdog snapdog-client
    install_ "$new_server" "$new_client"
    state snapdog masked inactive
    state snapdog-client masked inactive
    step "purge: the administrator's mask is not ours to remove"
    purge snapdog snapdog-client
    present /etc/systemd/system/snapdog.service
    present /etc/systemd/system/snapdog-client.service
}

scenario_reinstall_after_remove_restores_a_running_server() {
    install_ "$new_server" "$new_client"
    start_server
    state snapdog enabled active
    step "remove both"
    remove snapdog snapdog-client
    state snapdog masked inactive
    state snapdog-client masked inactive
    present "$note"
    step "install both again"
    install_ "$new_server" "$new_client"
    state snapdog enabled active
    state snapdog-client enabled active
    absent "$note" "$note.pending"
}

# The note of the removal must survive a reboot: /run is emptied, /var/lib is not.
scenario_reboot_reinstall_after_remove_restores_a_running_server() {
    if [ "${3:-}" = 1 ]; then
        install_ "$new_server" "$new_client"
        start_server
        state snapdog enabled active
        step "remove both, then reboot"
        remove snapdog snapdog-client
        present "$note"
    else
        step "after the reboot: install both again"
        install_ "$new_server" "$new_client"
        state snapdog enabled active
        state snapdog-client enabled active
        absent "$note" "$note.pending"
    fi
}

scenario_reinstall_after_remove_leaves_a_stopped_server_alone() {
    install_ "$new_server" "$new_client"
    systemctl enable snapdog >/dev/null 2>&1
    step "remove and install again, the server enabled but stopped throughout"
    remove snapdog snapdog-client
    absent "$note" "$note.pending"
    install_ "$new_server" "$new_client"
    state snapdog enabled inactive
}

scenario_reinstall_after_remove_leaves_a_disabled_server_alone() {
    install_ "$new_server" "$new_client"
    systemctl start snapdog
    state snapdog disabled active
    step "remove while the server runs without being enabled, then install again"
    remove snapdog snapdog-client
    install_ "$new_server" "$new_client"
    state snapdog disabled inactive
    absent "$note" "$note.pending"
}

# A removal whose postrm fails stops the server and leaves the package half removed. What
# it leaves must not start the server later, when the administrator has stopped it again.
scenario_interrupted_remove_leaves_nothing_for_a_later_upgrade() {
    install_ "$new_server" "$new_client"
    start_server
    build_failing_postrm_deb "$new_server" /tmp/failing-postrm.deb
    reinstall /tmp/failing-postrm.deb
    state snapdog enabled active
    step "remove with a postrm that fails"
    touch /tmp/failrm
    if apt-get remove -y -qq snapdog >/tmp/apt.log 2>&1; then
        echo "  FAIL  the removal did not fail ($step_name)"
        failures=$((failures + 1))
    else
        echo "  ok    the removal failed, as set up"
    fi
    # The files are gone, the unit with them, and prerm has stopped the server.
    if retry is_inactive snapdog; then echo "  ok    snapdog is stopped"; else
        echo "  FAIL  snapdog is not stopped ($step_name)"
        failures=$((failures + 1))
    fi
    present "$note.pending"
    absent "$note"
    rm /tmp/failrm
    step "the package is put right again (the server stays stopped, no note is left)"
    reinstall "$new_server"
    state snapdog enabled inactive
    absent "$note" "$note.pending"
    step "a further upgrade does not start it either"
    reinstall "$new_server" "$new_client"
    state snapdog enabled inactive
}

# With a policy-rc.d that forbids everything (exit 101), the scripts may not stop the
# server. Purge must not delete its state under the running process.
scenario_policy_rc_d_101_keeps_state_of_a_running_server() {
    install_ "$new_server" "$new_client"
    start_server
    state snapdog enabled active
    printf '#!/bin/sh\nexit 101\n' >/usr/sbin/policy-rc.d
    chmod +x /usr/sbin/policy-rc.d
    s=$(pid snapdog)
    step "remove under a policy-rc.d that returns 101: the server keeps running"
    remove snapdog snapdog-client
    same_pid snapdog "$s"
    step "purge: the state is kept, with a message, while the server runs"
    apt-get purge -y -qq snapdog snapdog-client >/tmp/apt.log 2>&1 || { cat /tmp/apt.log; exit 1; }
    if grep -q "snapdog.service is still running" /tmp/apt.log; then
        echo "  ok    purge said why it kept /var/lib/snapdog"
    else
        echo "  FAIL  purge printed no message ($step_name)"
        failures=$((failures + 1))
    fi
    present /var/lib/private/snapdog
    same_pid snapdog "$s"
    step "once the server is stopped, nothing but the files by hand are left"
    systemctl stop snapdog
    rm -rf /var/lib/private/snapdog /var/lib/snapdog
    absent /var/lib/private/snapdog /var/lib/snapdog
}

scenario_failed_server_unit() {
    install_ "$new_server" "$new_client"
    mkdir -p /etc/systemd/system/snapdog.service.d
    printf '[Service]\nExecStart=\nExecStart=/bin/false\nRestart=no\n' \
        >/etc/systemd/system/snapdog.service.d/fail.conf
    systemctl daemon-reload
    systemctl enable snapdog >/dev/null 2>&1
    systemctl start snapdog || true
    state snapdog enabled failed
    step "upgrade with a failed server: the package still installs"
    reinstall "$new_server" "$new_client"
    state snapdog-client enabled active
    step "remove and purge with a failed server"
    remove snapdog snapdog-client
    purge snapdog snapdog-client
    absent /var/lib/private/snapdog /var/lib/snapdog /etc/snapdog
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
        /etc/default/snapdog-client \
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
    absent /var/lib/private/snapdog /var/lib/snapdog
}

scenario_purge_after_remove() {
    install_ "$new_server" "$new_client"
    start_server
    remove snapdog snapdog-client
    present "$note"
    step "purge after remove"
    purge snapdog snapdog-client
    absent /var/lib/private/snapdog /var/lib/snapdog
}

# 0.28.2-1 is the last release without maintainer scripts. An upgrade from it leaves the
# client as the administrator left it: nothing recorded whether it was ever enabled, so the
# client's postinst records the current state before the generated code decides.
scenario_upgrade_from_old_release() {
    step "install the old release (no maintainer scripts)"
    install_ "$old_server" "$old_client"
    state snapdog disabled inactive
    state snapdog-client disabled inactive
    step "first upgrade to the release with scripts: the client stays disabled"
    install_ "$new_server" "$new_client"
    state snapdog disabled inactive
    state snapdog-client disabled inactive
    absent /etc/systemd/system/multi-user.target.wants/snapdog-client.service
    step "a second upgrade does not enable it either"
    reinstall "$new_server" "$new_client"
    state snapdog disabled inactive
    state snapdog-client disabled inactive
    step "once the administrator enables it, upgrades keep it enabled and running"
    systemctl enable --now snapdog-client >/dev/null 2>&1
    c=$(pid snapdog-client)
    reinstall "$new_server" "$new_client"
    state snapdog-client enabled active
    changed_pid snapdog-client "$c"
}

scenario_upgrade_from_old_release_with_client_enabled_by_hand() {
    install_ "$old_server" "$old_client"
    systemctl enable --now snapdog-client >/dev/null 2>&1
    state snapdog-client enabled active
    c=$(pid snapdog-client)
    step "first upgrade with the client enabled and running by hand"
    install_ "$new_server" "$new_client"
    state snapdog-client enabled active
    changed_pid snapdog-client "$c"
    state snapdog disabled inactive
    c=$(pid snapdog-client)
    step "second upgrade"
    reinstall "$new_server" "$new_client"
    state snapdog-client enabled active
    changed_pid snapdog-client "$c"
}

scenario_upgrade_from_old_release_with_client_disabled_again() {
    install_ "$old_server" "$old_client"
    systemctl enable --now snapdog-client >/dev/null 2>&1
    systemctl disable --now snapdog-client >/dev/null 2>&1
    state snapdog-client disabled inactive
    step "first upgrade with the client enabled once and disabled again"
    install_ "$new_server" "$new_client"
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
    state snapdog-client disabled inactive
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

# Removed without being purged, an old release leaves its version in dpkg's records, so
# installing the new one counts as an upgrade from it.
scenario_install_after_removing_old_release() {
    install_ "$old_server" "$old_client"
    remove snapdog snapdog-client
    step "install the new release after removing (not purging) the old one"
    install_ "$new_server" "$new_client"
    state snapdog disabled inactive
    state snapdog-client disabled inactive
}

scenario_upgrade_from_old_release_with_masked_server() {
    install_ "$old_server" "$old_client"
    systemctl mask snapdog >/dev/null 2>&1
    state snapdog masked inactive
    step "first upgrade with the server masked beforehand"
    install_ "$new_server" "$new_client"
    state snapdog masked inactive
    state snapdog-client disabled inactive
}

echo "== ${2}"
"scenario_${2}" "$@"
exit "$((failures > 0))"
