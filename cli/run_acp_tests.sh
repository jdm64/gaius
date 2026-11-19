#!/usr/bin/env bash
#
# install acp-tck with:
# - uv tool install git+https://github.com/agentclientprotocol/acp-tck@v0.4.0
#

acp-tck \
  --protocol-version 1 \
  --agent-cwd /tmp/acp-tck-wd \
  --timeout 90 \
  --test-timeout 300 \
  --report-json report.json \
  -- $(pwd)/target/debug/gaius acp
