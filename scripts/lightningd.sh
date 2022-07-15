#!/usr/bin/env bash

set -u

source ./scripts/lib.sh

# Starts Bitcoin and 2 LN nodes, opening a channel between the LN nodes
POLL_INTERVAL=1
RUN_GATEWAY=$1

if [[ "$(lightningd --bitcoin-cli "$(which false)" --dev-no-plugin-checksum 2>&1 )" =~ .*"--dev-no-plugin-checksum: unrecognized option".* ]]; then
  LIGHTNING_FLAGS="--dev-fast-gossip --dev-bitcoind-poll=1"
else
  LIGHTNING_FLAGS=""
fi

if [[ "$RUN_GATEWAY" == 1 ]]; then
  PLUGIN_FLAGS="--plugin=$FM_BIN_DIR/ln_gateway --minimint-cfg=$FM_CFG_DIR"
else
  PLUGIN_FLAGS=""
fi

# Start lightning nodes
lightningd $LIGHTNING_FLAGS --network regtest --bitcoin-rpcuser=bitcoin --bitcoin-rpcpassword=bitcoin --lightning-dir=$FM_LN1_DIR --addr=127.0.0.1:9000 $PLUGIN_FLAGS &
echo $! >> $FM_PID_FILE
lightningd $LIGHTNING_FLAGS --network regtest --bitcoin-rpcuser=bitcoin --bitcoin-rpcpassword=bitcoin --lightning-dir=$FM_LN2_DIR --addr=127.0.0.1:9001 &
echo $! >> $FM_PID_FILE
until [ -e $FM_LN1_DIR/regtest/lightning-rpc ]; do
    sleep $POLL_INTERVAL
done
until [ -e $FM_LN2_DIR/regtest/lightning-rpc ]; do
    sleep $POLL_INTERVAL
done

# Initialize wallet and get ourselves some money
mine_blocks 101

# Open channel
open_channel