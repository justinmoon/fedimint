#!/usr/bin/env bash

set -euo pipefail

pkill -9 fedimintd lnd lightningd gatewayd devimint esplora electrs bitcoind faucet || true

if [[ -z "${IN_NIX_SHELL:-}" ]]; then
  echo "It is recommended to run this command from a Nix dev shell. Use 'nix develop' first"
  sleep 3
fi

# Flag to enable verbose build output from depndent processes (disabled by default)
export FM_VERBOSE_OUTPUT=0

source scripts/build.sh

rm target/logs || true
ln -s $FM_LOGS_DIR target/logs

pushd ../fedi
cargo build --bin fedimintd
popd

mkdir mybin || true
pwd
cp ../fedi/target/debug/fedimintd mybin

export PATH=$PWD/mybin:$PATH
mkdir -p $FM_LOGS_DIR
devimint dev-fed 2>$FM_LOGS_DIR/devimint-outer.log &
echo $! >> $FM_PID_FILE
eval "$(devimint env)"

mprocs -c misc/mprocs.yaml
