#!/usr/bin/env bash
# run gateway client tests

export FM_GATEWAY_API_ADDR=http://127.0.0.1:8175 
export FM_GATEWAY_FEES=0,0
cargo test -p ln-gateway test_gateway_client
