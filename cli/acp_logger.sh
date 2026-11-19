#!/usr/bin/env bash
#
# Description:
#  Used to capture traffic from the ACP connection for debugging.
#  Logs are stored next to the agent binary or use ACP_LOG_DIR env var.
#
# Usage:
#  Set this as the command the ACP client runs
#  /path/to/acp_logger.sh /path/to/gaius acp
#

set -euo pipefail

REAL_AGENT="$1"
shift

AGENT_DIR=$(dirname $REAL_AGENT)
LOG_DIR="${ACP_LOG_DIR:-$AGENT_DIR/acp-logs}"
TS=$(date +%Y%m%d-%H%M%S)
SES_DIR="$LOG_DIR/session-${TS}-$$"

mkdir -p "$SES_DIR"
IN_LOG="${SES_DIR}/in"
OUT_LOG="${SES_DIR}/out"
ERR_LOG="${SES_DIR}/err"
META="${SES_DIR}/meta"

{
  echo "start=$(date -Iseconds)"
  echo "agent=$REAL_AGENT"
  echo "args=$*"
  echo "pid=$$"
} > "$META"

stdbuf -oL -eL "$REAL_AGENT" "$@" \
  < <(stdbuf -oL tee -a "$IN_LOG") \
  > >(stdbuf -oL tee -a "$OUT_LOG") \
  2> >(stdbuf -oL tee -a "$ERR_LOG" >&2)

echo "end=$(date -Iseconds)" >> "$META"
