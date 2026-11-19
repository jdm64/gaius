#!/usr/bin/env bash
#
# install acp-tck with:
# - uv tool install git+https://github.com/agentclientprotocol/acp-tck@v0.4.0
#
# usage: run_acp_tests.sh [protocol-version]
# protocol-version defaults to 1 if not given.
#

protocol_version="${1:-1}"
CWD="/tmp/acp-tck-wd"

mkdir -p $CWD

acp-tck \
  --protocol-version "$protocol_version" \
  --agent-cwd $CWD \
  --timeout 90 \
  --test-timeout 300 \
  --report-json report.json \
  -- $(pwd)/target/debug/gaius acp
