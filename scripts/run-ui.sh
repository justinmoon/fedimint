#!/usr/bin/env bash

export FM_FED_SIZE=${1:-2}

source scripts/build.sh $FM_FED_SIZE

export RUST_BACKTRACE=1

pushd ../fedi/fedimintd || exit
cargo build --bin fedimintd
popd || return

mkdir $FM_CFG_DIR/server-0
mkdir $FM_CFG_DIR/server-1

start_bitcoind | show_verbose_output &
start_federation

echo "UI instances running at http://127.0.0.1:18185 and http://127.0.0.1:18175"

# Allow daemons to keep running
wait
