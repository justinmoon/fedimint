#!/usr/bin/env bash
# Runs a CLI-based integration test

pkill -9 distributedgen fedimintd bitcoind lnd lightningd gatewayd fixtures esplora electrs fedimint_bin_tests devimint fedimint-bin-tests mprocs || true

set -euo pipefail
export RUST_LOG="${RUST_LOG:-info}"
source ./scripts/build.sh

rm target/logs || true
ln -s $FM_LOGS_DIR target/logs
rm target/test || true
ln -s $FM_LOGS_DIR target/test

devimint external-daemons 2>/dev/null &
echo $! >> $FM_PID_FILE

STATUS=$(devimint wait)
if [ "$STATUS" = "ERROR" ]
then
    echo "base daemons didn't start correctly"
    exit 1
fi

eval "$(devimint env)"

# use real daemons for gatewayd tests
export FM_TEST_USE_REAL_DAEMONS=1
cargo test -p ln-gateway ${CARGO_PROFILE:+--profile ${CARGO_PROFILE}} gatewayd_

# use fake daemons for gateway client tests
export FM_TEST_USE_REAL_DAEMONS=0
cargo test -p ln-gateway test_gateway_client

echo "Logs -> $FM_LOGS_DIR"
