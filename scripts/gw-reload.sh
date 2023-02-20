#!/usr/bin/env bash
# Reload core-lightning gateway plugin

set -euo pipefail

cargo build ${CARGO_PROFILE:+--profile ${CARGO_PROFILE}} --bin gateway

$FM_CLN plugin stop gateway &> /dev/null || true
$FM_CLN -k plugin subcommand=start plugin=$FM_BIN_DIR/gateway fedimint-cfg=$FM_CFG_DIR &> /dev/null

echo "Gateway plugin reloaded"
