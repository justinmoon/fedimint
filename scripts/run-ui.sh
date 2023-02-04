#!/usr/bin/env bash

pkill fedimintd

export FM_FED_SIZE=${1:-2}
# clear out federation startup configs folder
rm -r $PWD/fed-ui
mkdir $PWD/fed-ui

# start bitcoind on regtest in the background
bitcoind -regtest -daemon -fallbackfee=0.0004 -txindex -server -rpcuser=bitcoin -rpcpassword=bitcoin

# start guardians
export API_BIND=0.0.0.0:10000
export P2P_BIND=0.0.0.0:10001
export API_URL=wss://0.0.0.0:10000
export P2P_URL=wss://0.0.0.0:10001
ID=0
mkdir $PWD/fed-ui/mock-$ID
cargo run --bin fedimintd $PWD/fed-ui/mock-$ID pw-$ID --listen-ui 127.0.0.1:$((19800 + $ID)) &

export API_BIND=0.0.0.0:20000
export P2P_BIND=0.0.0.0:20001
export API_URL=wss://0.0.0.0:20000
export P2P_URL=wss://0.0.0.0:20001
ID=1
mkdir $PWD/fed-ui/mock-$ID
cargo run --bin fedimintd $PWD/fed-ui/mock-$ID pw-$ID --listen-ui 127.0.0.1:$((19800 + $ID)) &

function kill_fedimint_processes {
  pkill "fedimintd" || true
}

trap kill_fedimint_processes EXIT
