#!/usr/bin/env bash

bitcoind -regtest -daemon -fallbackfee=0.0004 -txindex -server -rpcuser=bitcoin -rpcpassword=bitcoin -zmqpubrawblock=tcp://127.0.0.1:28332 -zmqpubrawtx=tcp://127.0.0.1:28333

lnd --noseedbackup --lnddir=$PWD/lnd1 &

# lncli -n regtest --lnddir=$PWD/lnd1 create

lnd --noseedbackup --lnddir=$PWD/lnd2 &
