#!/usr/bin/env bash

source ./scripts/lib.sh
echo "Run with 'source ./scripts/build.sh [fed_size] [dir]"

# allow for overriding arguments
export FM_FED_SIZE=${1:-4}
export FM_DIR=${2-"$(mktemp -d)"}
echo "Setting up env variables in $FM_DIR"

# Builds the rust executables and sets environment variables
SRC_DIR="$( cd -- "$( dirname -- "${BASH_SOURCE[0]}" )/.." &> /dev/null && pwd )"
cd $SRC_DIR
cargo build --release

# Define temporary directories to not overwrite manually created config if run locally
export FM_TEST_DIR=$FM_DIR
export FM_BIN_DIR="$SRC_DIR/target/release"
export FM_PID_FILE="$FM_DIR/.pid"
export FM_LN1_DIR="$FM_TEST_DIR/ln1"
export FM_LN2_DIR="$FM_TEST_DIR/ln2"
export FM_BTC_DIR="$FM_TEST_DIR/bitcoin"
export FM_CFG_DIR="$FM_TEST_DIR/cfg"
mkdir -p $FM_LN1_DIR
mkdir -p $FM_LN2_DIR
mkdir -p $FM_BTC_DIR
mkdir -p $FM_CFG_DIR

# Generate federation client config
$FM_BIN_DIR/configgen -- $FM_CFG_DIR $FM_FED_SIZE 4000 5000 1000 10000 100000 1000000 10000000

# Define clients
export FM_LN1="lightning-cli --network regtest --lightning-dir=$FM_LN1_DIR"
export FM_LN2="lightning-cli --network regtest --lightning-dir=$FM_LN2_DIR"
export FM_BTC_CLIENT="bitcoin-cli -regtest -rpcuser=bitcoin -rpcpassword=bitcoin"
export FM_MINT_CLIENT="$FM_BIN_DIR/mint-client-cli $FM_CFG_DIR"
export FM_MINT_RPC_CLIENT="$FM_BIN_DIR/mint-rpc-client"
export FM_CLIENTD="$FM_BIN_DIR/clientd"
export FM_CLIENTD_CLI="$FM_BIN_DIR/clientd-cli"

# Alias clients
alias ln1="\$FM_LN1"
alias ln2="\$FM_LN2"
alias btc_client="\$FM_BTC_CLIENT"
alias mint_client="\$FM_MINT_CLIENT"
alias mint_rpc_client="\$FM_MINT_RPC_CLIENT"
alias clientd="\$FM_CLIENTD"
alias clientd-cli="\$FM_CLIENTD_CLI"

trap kill_minimint_processes EXIT