#!/usr/bin/env bash
# Runs a CLI-based integration test

set -euxo pipefail
export RUST_LOG=info

export PEG_IN_AMOUNT=10000
source ./scripts/setup-tests.sh
./scripts/start-fed.sh
./scripts/pegin.sh # peg in user

export PEG_IN_AMOUNT=99999

# start_gateway
export RUST_BACKTRACE=1
$FM_BIN_DIR/gateway_lnd localhost 11010 $FM_LND1_DIR/tls.cert $FM_LND1_DIR/data/chain/bitcoin/regtest/admin.macaroon $FM_CFG_DIR &
echo $! >> $FM_PID_FILE
sleep 1

./scripts/pegin.sh $PEG_IN_AMOUNT 1 # peg in gateway

# start 

#### BEGIN TESTS ####

# reissue
# TOKENS=$($FM_MINT_CLIENT spend '42000msat' | jq -r '.token')
# [[ $($FM_MINT_CLIENT info | jq -r '.total_amount') = "9958000" ]]
# $FM_MINT_CLIENT validate $TOKENS
# $FM_MINT_CLIENT reissue $TOKENS
# $FM_MINT_CLIENT fetch

# peg out
# PEG_OUT_ADDR="$($FM_BTC_CLIENT getnewaddress)"
# $FM_MINT_CLIENT peg-out $PEG_OUT_ADDR 500
# sleep 5 # FIXME wait for tx to be included
# await_block_sync
# until [ "$($FM_BTC_CLIENT getreceivedbyaddress $PEG_OUT_ADDR 0)" == "0.00000500" ]; do
#   sleep $POLL_INTERVAL
# done
# mine_blocks 10
# RECEIVED=$($FM_BTC_CLIENT getreceivedbyaddress $PEG_OUT_ADDR)
# [[ "$RECEIVED" = "0.00000500" ]]

# outgoing lightning
INVOICE="$($FM_LN2 invoice 100000 test test 1m | jq -r '.bolt11')"
await_cln_block_processing
$FM_MINT_CLIENT ln-pay $INVOICE
# Check that ln-gateway has received the ecash notes from the user payment
# 100,000 sats + 100 sats without processing fee
LN_GATEWAY_BALANCE="$($FM_GATEWAY_CLI balance | jq -r '.balance_msat')"
[[ "$LN_GATEWAY_BALANCE" = "100100000" ]]
INVOICE_RESULT="$($FM_LN2 waitinvoice test)"
INVOICE_STATUS="$(echo $INVOICE_RESULT | jq -r '.status')"
[[ "$INVOICE_STATUS" = "paid" ]]

# incoming lightning
INVOICE="$($FM_MINT_CLIENT ln-invoice '100000msat' 'integration test' | jq -r '.invoice')"
INVOICE_RESULT=$($FM_LN2 pay $INVOICE)
INVOICE_STATUS="$(echo $INVOICE_RESULT | jq -r '.status')"
[[ "$INVOICE_STATUS" = "complete" ]]
