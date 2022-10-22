#!/usr/bin/env bash

set -u

FM_FED_SIZE=${1:-4}

source ./scripts/build.sh $FM_FED_SIZE

# Starts Bitcoin and 2 LN nodes, opening a channel between the LN nodes
POLL_INTERVAL=1

# Start bitcoind and wait for it to become ready
bitcoind -regtest -fallbackfee=0.0004 -txindex -server -rpcuser=bitcoin -rpcpassword=bitcoin -datadir=$FM_BTC_DIR &
echo $! >> $FM_PID_FILE

until [ "$($FM_BTC_CLIENT getblockchaininfo | jq -r '.chain')" == "regtest" ]; do
  sleep $POLL_INTERVAL
done
$FM_BTC_CLIENT createwallet ""

if [[ "$(lightningd --bitcoin-cli "$(which false)" --dev-no-plugin-checksum 2>&1 )" =~ .*"--dev-no-plugin-checksum: unrecognized option".* ]]; then
  LIGHTNING_FLAGS=""
else
  LIGHTNING_FLAGS="--dev-fast-gossip --dev-bitcoind-poll=1"
fi

# Start core-lightning nodes
lightningd $LIGHTNING_FLAGS --network regtest --bitcoin-rpcuser=bitcoin --bitcoin-rpcpassword=bitcoin --lightning-dir=$FM_LN1_DIR --addr=127.0.0.1:9000 &
echo $! >> $FM_PID_FILE
lightningd $LIGHTNING_FLAGS --network regtest --bitcoin-rpcuser=bitcoin --bitcoin-rpcpassword=bitcoin --lightning-dir=$FM_LN2_DIR --addr=127.0.0.1:9001 &
echo $! >> $FM_PID_FILE
# FIXME: could this just be await_cln_block_processing?
until [ -e $FM_LN1_DIR/regtest/lightning-rpc ]; do
    sleep $POLL_INTERVAL
done
until [ -e $FM_LN2_DIR/regtest/lightning-rpc ]; do
    sleep $POLL_INTERVAL
done

# Start lnd nodes
cp $PWD/scripts/lnd1.conf $FM_LND1_DIR/lnd.conf
cp $PWD/scripts/lnd2.conf $FM_LND2_DIR/lnd.conf

lnd --noseedbackup --lnddir=$PWD/lnd1 &
lnd --noseedbackup --lnddir=$PWD/lnd2 &

await_lnd_block_processing

# Initialize wallet and get ourselves some money
mine_blocks 101

# Open channel
open_channel
