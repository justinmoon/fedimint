#!/usr/bin/env bash

# start setup
# set -euxo pipefail
export RUST_LOG=info

FM_FED_SIZE=${1:-4}

source ./scripts/build.sh $FM_FED_SIZE

# Starts Bitcoin and 2 LN nodes, opening a channel between the LN nodes
POLL_INTERVAL=1

# Start bitcoind and wait for it to become ready
bitcoind -regtest -fallbackfee=0.0004 -txindex -server -rpcuser=bitcoin -rpcpassword=bitcoin -datadir=$FM_BTC_DIR -zmqpubrawblock=tcp://127.0.0.1:28332 -zmqpubrawtx=tcp://127.0.0.1:28333 &
echo $! >> $FM_PID_FILE

until [ "$($FM_BTC_CLIENT getblockchaininfo | jq -r '.chain')" == "regtest" ]; do
  sleep $POLL_INTERVAL
done
$FM_BTC_CLIENT createwallet ""


# end setup

# TODO: put these data dirs in /tmp
# setup lnd1 datadir
rm -rf $PWD/lnd1
mkdir $PWD/lnd1
cp $PWD/scripts/lnd1.conf $PWD/lnd1/lnd.conf

# setup lnd2 datadir
rm -rf $PWD/lnd2
mkdir $PWD/lnd2
cp $PWD/scripts/lnd2.conf $PWD/lnd2/lnd.conf

# bitcoind -regtest -daemon -fallbackfee=0.0004 -txindex -server -rpcuser=bitcoin -rpcpassword=bitcoin -zmqpubrawblock=tcp://127.0.0.1:28332 -zmqpubrawtx=tcp://127.0.0.1:28333

# start lightning nodes
lnd --noseedbackup --lnddir=$PWD/lnd1 &
lnd --noseedbackup --lnddir=$PWD/lnd2 &
mine_blocks 102  # lnd seems to want blocks mined in order to sync chain and startup rpc servers


FM_LNCLI1="lncli -n regtest --lnddir=$PWD/lnd1 --rpcserver=localhost:11010"
FM_LNCLI2="lncli -n regtest --lnddir=$PWD/lnd2 --rpcserver=localhost:11009"

function await_lnd_block_processing() {

  # ln1
  until [ "true" == "$($FM_LNCLI1 getinfo | jq -r '.synced_to_chain')" ]
  do
    sleep $POLL_INTERVAL
  done

  # ln2
  until [ "true" == "$($FM_LNCLI2 getinfo | jq -r '.synced_to_chain')" ]
  do
    sleep $POLL_INTERVAL
  done
}

echo "WAITING FOR LND STARTUP"
await_lnd_block_processing

open_lnd_channel
mine_blocks 10

# send funds from ln1 to ln2 via rust
INVOICE=$($FM_LNCLI2 addinvoice -amt 100 | jq -r ".payment_request")
echo "invoice $INVOICE"
cargo run --bin lnd_test http://localhost:11010 $PWD/lnd1/tls.cert $PWD/lnd1/data/chain/bitcoin/regtest/admin.macaroon $INVOICE

pkill "lnd" 2>&1 /dev/null
pkill "bitcoind" 2>&1 /dev/null
