#!/usr/bin/env bash
#
# Issue #532 at scale: 100 000 live MQTT sessions against a three-node broadcast
# cluster, everything in docker compose. Linux only — the case itself skips on
# Windows (rmqtt-test/src/tests/functional/issue532_live_cluster.rs explains why).
#
#   ./run.sh                        # ramp to 100000 sessions at 500/s, hold 60s
#   RMQTT_LIVE_TOTAL=30000 ./run.sh # smaller first run
#   NODE_CPUS=1 ./run.sh            # tighter brokers (closer to a small pod)
#
# Works with the `docker compose` plugin and with the standalone
# `docker-compose` binary (v1 or v2); the CPU/memory caps are dropped
# automatically on a compose that cannot express them.
#
# Artifacts written next to this script:
#   issue532-live.log        harness output (the verdict and its evidence)
#   issue532-live-nodes.log  the three brokers' logs
#
# Exit codes: 0 = the case passed, 1 = it failed (a stall was reproduced),
# 2 = infrastructure error (see the log), 3 = the case was skipped (setup wrong).

set -euo pipefail
cd "$(dirname "$0")"

LOG=issue532-live.log
NODES_LOG=issue532-live-nodes.log

if [ "$(uname -s)" != "Linux" ]; then
    echo "!! this stack is Linux-only (the case skips elsewhere)" >&2
    exit 2
fi

# --- which compose? plugin first, then the standalone binary ----------------
if docker compose version >/dev/null 2>&1; then
    C=(docker compose)
elif command -v docker-compose >/dev/null 2>&1; then
    C=(docker-compose)
else
    echo "!! neither 'docker compose' nor 'docker-compose' is available" >&2
    exit 2
fi
# No pipe here on purpose: `set -o pipefail` plus a reader that exits early
# (head) can turn a harmless SIGPIPE into an abort.
ver="$("${C[@]}" version 2>&1 || true)"
ver="${ver%%$'\n'*}"
echo "==> using: ${C[*]} (${ver})"

# --- compose files: base, plus the caps if they parse -----------------------
F=(-f docker-compose.yml)
if ! "${C[@]}" "${F[@]}" config >/dev/null 2>&1; then
    echo "!! ${C[*]} cannot parse docker-compose.yml:" >&2
    "${C[@]}" "${F[@]}" config >&2 || true
    exit 2
fi
if "${C[@]}" "${F[@]}" -f docker-compose.limits.yml config >/dev/null 2>&1; then
    F+=(-f docker-compose.limits.yml)
    echo "==> cpu/memory caps: node ${NODE_CPUS:-2} cpu / ${NODE_MEM:-2g}, loadgen ${LOADGEN_CPUS:-8} cpu / ${LOADGEN_MEM:-16g}"
else
    echo "!! note: this compose cannot apply cpu/memory caps (docker-compose.limits.yml) -"
    echo "!!       continuing without them, so NODE_CPUS/LOADGEN_CPUS have no effect here" >&2
fi

echo "==> building the runtime image"
"${C[@]}" "${F[@]}" build

echo "==> compiling rmqttd and mqtt_harness in the build image (first run: several minutes)"
"${C[@]}" "${F[@]}" run --rm build

echo "==> starting the three brokers"
"${C[@]}" "${F[@]}" up -d node1 node2 node3

# Wait for the primary listeners: the case would otherwise report "cluster not
# reachable", which is a setup error and not the incident.
for port in 1920 1922 1924; do
    ready=""
    for _ in $(seq 1 60); do
        if (exec 3<>"/dev/tcp/127.0.0.1/${port}") 2>/dev/null; then
            exec 3>&-
            ready=1
            break
        fi
        sleep 1
    done
    if [ -z "${ready}" ]; then
        echo "!! a node listening on ${port} did not come up; broker logs:" >&2
        "${C[@]}" "${F[@]}" logs node1 node2 node3 >&2 || true
        "${C[@]}" "${F[@]}" down || true
        exit 2
    fi
done

echo "==> running the load (RMQTT_LIVE_TOTAL=${RMQTT_LIVE_TOTAL:-100000}, RMQTT_LIVE_RAMP_RATE=${RMQTT_LIVE_RAMP_RATE:-500}, RMQTT_LIVE_HOLD_SECS=${RMQTT_LIVE_HOLD_SECS:-60})"
set +e
"${C[@]}" "${F[@]}" run --rm loadgen 2>&1 | tee "${LOG}"
rc=${PIPESTATUS[0]}
set -e

echo "==> collecting broker logs"
"${C[@]}" "${F[@]}" logs node1 node2 node3 > "${NODES_LOG}" 2>&1 || true

echo "==> stopping the stack (the cargo volumes are kept, so the next build is incremental)"
"${C[@]}" "${F[@]}" down >/dev/null 2>&1 || true

if grep -q "Skipped: 1" "${LOG}"; then
    echo "!! the case was SKIPPED — the setup is wrong, not the broker. Reason is in ${LOG}." >&2
    exit 3
fi
if grep -q "Errors: 1" "${LOG}"; then
    echo "!! infrastructure error (the load could not be built, or a node was unreachable). See ${LOG}." >&2
    exit 2
fi
if [ "${rc}" -ne 0 ]; then
    echo "!! the case FAILED — a stall was reproduced. Evidence: ${LOG}, broker logs: ${NODES_LOG}." >&2
    exit 1
fi

echo "==> done: the 100K-session cluster stayed healthy. ${LOG} has the evidence."
