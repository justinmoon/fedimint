#!/usr/bin/env bash
# Runs the all the Rust integration tests

set -euo pipefail
export RUST_LOG="${RUST_LOG:-info,timing=debug}"
# export RUST_LOG=info,ln-gateway=debug,client=trace,fedimint-ln-client=debug,jsonrpsee=trace
# export RUST_LOG=info,ln-gateway=debug,fedimint-client=debug,fedimint-ln-client=debug,jsonrpsee=trace
export RUST_LOG=info

source scripts/build.sh

>&2 echo "### Setting up tests"

devimint external-daemons &
echo $! >> $FM_PID_FILE

STATUS=$(devimint wait)
if [ "$STATUS" = "ERROR" ]
then
    echo "base daemons didn't start correctly"
    exit 1
fi

eval "$(devimint env)"
>&2 echo "### Setting up tests - complete"

export FM_TEST_USE_REAL_DAEMONS=1

export FM_GATEWAY_FEES="0,0"
export RUST_BACKTRACE=1

env RUST_BACKTRACE=1 cargo test -p ln-gateway test_gateway_client
