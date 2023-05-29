#!/usr/bin/env bash
# run gateway client tests

export RUST_BACKTRACE=1
export RUST_LOG=info
cargo test -p ln-gateway test_gateway_client
