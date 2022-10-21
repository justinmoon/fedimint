#!/usr/bin/env bash


rm -rf $PWD/lnd1
mkdir $PWD/lnd1
cp $PWD/scripts/lnd1.conf $PWD/lnd1/lnd.conf

rm -rf $PWD/lnd2
mkdir $PWD/lnd2
cp $PWD/scripts/lnd2.conf $PWD/lnd2/lnd.conf

bitcoind -regtest -daemon -fallbackfee=0.0004 -txindex -server -rpcuser=bitcoin -rpcpassword=bitcoin -zmqpubrawblock=tcp://127.0.0.1:28332 -zmqpubrawtx=tcp://127.0.0.1:28333

lnd --noseedbackup --lnddir=$PWD/lnd1 &

# lncli -n regtest --lnddir=$PWD/lnd1 create

lnd --noseedbackup --lnddir=$PWD/lnd2 &
