#!/usr/bin/env bash

set -u

echo "Setting up tests..."

FM_FED_SIZE=${1:-4}

pkill -9 lightningd lnd bitcoind intercept_htlcs

source ./scripts/build.sh $FM_FED_SIZE

# Starts Bitcoin and 2 LN nodes, opening a channel between the LN nodes
POLL_INTERVAL=1

# Start bitcoind and wait for it to become ready
bitcoind -regtest -fallbackfee=0.00001 -txindex -server -rpcuser=bitcoin -rpcpassword=bitcoin -datadir=$FM_BTC_DIR &
echo $! >> $FM_PID_FILE

# Required by tests to manipulate bitcoind
export FM_TEST_BITCOIND_RPC="http://bitcoin:bitcoin@127.0.0.1:18443"
export FM_BITCOIND_RPC="http://bitcoin:bitcoin@127.0.0.1:18443"

until [ "$($FM_BTC_CLIENT getblockchaininfo | jq -e -r '.chain')" == "regtest" ]; do
  sleep $POLL_INTERVAL
done
$FM_BTC_CLIENT createwallet ""

if [[ "$(lightningd --bitcoin-cli "$(which false)" --dev-no-plugin-checksum 2>&1 )" =~ .*"--dev-no-plugin-checksum: unrecognized option".* ]]; then
  LIGHTNING_FLAGS=""
else
  LIGHTNING_FLAGS="--dev-fast-gossip --dev-bitcoind-poll=1"
fi

# Start CLN 1 & 2
lightningd $LIGHTNING_FLAGS --network regtest --bitcoin-rpcuser=bitcoin --bitcoin-rpcpassword=bitcoin --lightning-dir=$FM_CLN_DIR --addr=127.0.0.1:9000 &
lightningd $LIGHTNING_FLAGS --network regtest --bitcoin-rpcuser=bitcoin --bitcoin-rpcpassword=bitcoin --lightning-dir=$FM_CLN2_DIR --addr=127.0.0.1:9001 &

# Wait for CLN nodes to start
echo "awaiting cln"
await_cln_start

# Start LND node
lnd --lnddir=$FM_LND_DIR &
echo $! >> $FM_PID_FILE
echo "awaiting lnd"
await_lnd_start

# Initialize wallet and get ourselves some money
echo "mining blocks"
mine_blocks 101

# Open channel
open_channels

# arbitrary sleep
sleep 3

# Run HTLC interceptor
echo "running htlc interceptor"
# cargo run --bin intercept_htlcs $FM_LND_RPC_ADDR $FM_LND_TLS_CERT $FM_LND_MACAROON &
$FM_BIN_DIR/intercept_htlcs $FM_LND_RPC_ADDR $FM_LND_TLS_CERT $FM_LND_MACAROON &
echo $! >> $FM_PID_FILE

# arbitrary sleep
sleep 3

# cln1 pays cln2
echo "paying invoice"
INVOICE="$($FM_CLN2 invoice 42000 lnd-to-cln test 1m | jq -e -r '.bolt11')"
$FM_CLN pay "$INVOICE"
# INVOICE_STATUS="$($FM_CLN2 waitinvoice lnd-to-cln | jq -e -r '.status')"
INVOICE_STATUS="$($FM_CLN2 waitinvoice lnd-to-cln)"
echo $INVOICE_STATUS
