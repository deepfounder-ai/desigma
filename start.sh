#!/usr/bin/env bash
# fusion-desigma lifecycle script for fusion-supervisor.
#
#   start.sh start | stop | restart | status | log
#
# fusion-supervisor's default start path is `bash $FUSION_ROOT/<repo>/start.sh`,
# so this script follows the monorepo lifecycle convention (start|stop|status|log,
# pidfile, SIGKILL fallback) the same way fusion-design's start.sh does.
#
# fusion-desigma is a GUI desktop app: "start" launches the editor via the
# fusion shell launcher, "status" reports the pid state, "log" tails the run log.
# There is no HTTP face, so health probing is process liveness only.
set -euo pipefail

root=$(cd "$(dirname "$0")" && pwd)
pidfile="$root/.fusion-desigma.pid"
logfile="$root/logs/fusion-desigma.log"
mkdir -p "$root/logs"

pid_of() {
    [ -f "$pidfile" ] && cat "$pidfile" 2>/dev/null || true
}

is_running() {
    local pid
    pid=$(pid_of)
    [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null
}

do_start() {
    if is_running; then
        echo "fusion-desigma: already running (pid $(pid_of))"
        return 0
    fi
    rm -f "$pidfile"
    echo "fusion-desigma: starting"
    nohup "$root/bin/fusion-desigma" >>"$logfile" 2>&1 &
    echo $! >"$pidfile"
    sleep 1
    if is_running; then
        echo "fusion-desigma: started (pid $(pid_of), log $logfile)"
    else
        echo "fusion-desigma: failed to start; see $logfile" >&2
        rm -f "$pidfile"
        return 1
    fi
}

do_stop() {
    local pid
    pid=$(pid_of)
    if [ -z "$pid" ]; then
        echo "fusion-desigma: not running (no pidfile)"
        return 0
    fi
    if kill -0 "$pid" 2>/dev/null; then
        echo "fusion-desigma: stopping pid $pid"
        kill "$pid" 2>/dev/null || true
        for _ in 1 2 3 4 5; do
            kill -0 "$pid" 2>/dev/null || break
            sleep 1
        done
        if kill -0 "$pid" 2>/dev/null; then
            echo "fusion-desigma: pid $pid did not exit, sending SIGKILL"
            kill -9 "$pid" 2>/dev/null || true
        fi
    fi
    rm -f "$pidfile"
    echo "fusion-desigma: stopped"
}

do_status() {
    if is_running; then
        echo "fusion-desigma: running (pid $(pid_of))"
        return 0
    fi
    echo "fusion-desigma: stopped"
    return 1
}

do_log() {
    tail -n 50 "$logfile" 2>/dev/null || echo "fusion-desigma: no log yet at $logfile"
}

case "${1:-}" in
    start) do_start ;;
    stop) do_stop ;;
    restart) do_stop; do_start ;;
    status) do_status ;;
    log) do_log ;;
    *) echo "usage: $0 {start|stop|restart|status|log}" >&2; exit 2 ;;
esac
