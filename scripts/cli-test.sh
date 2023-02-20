#!/usr/bin/env bash
# Runs a CLI-based integration test

set -euxo pipefail
export RUST_LOG=info

export PEG_IN_AMOUNT=10000
source ./scripts/setup-tests.sh

# Test config en/decryption tool
export FM_PASSWORD=pass0
$FM_DISTRIBUTEDGEN config-decrypt --in-file $FM_CFG_DIR/server-0/private.encrypt --out-file $FM_CFG_DIR/server-0/config-plaintext.json
export FM_PASSWORD=pass-foo
$FM_DISTRIBUTEDGEN config-encrypt --in-file $FM_CFG_DIR/server-0/config-plaintext.json --out-file $FM_CFG_DIR/server-0/config-2
$FM_DISTRIBUTEDGEN config-decrypt --in-file $FM_CFG_DIR/server-0/config-2 --out-file $FM_CFG_DIR/server-0/config-plaintext-2.json
cmp --silent $FM_CFG_DIR/server-0/config-plaintext.json $FM_CFG_DIR/server-0/config-plaintext-2.json

./scripts/start-fed.sh
./scripts/pegin.sh # peg in user

export PEG_IN_AMOUNT=99999
start_gatewayd
# ./scripts/pegin.sh $PEG_IN_AMOUNT CLN # peg in CLN gateway
./scripts/pegin.sh $PEG_IN_AMOUNT LND # peg in LND gateway

#### BEGIN TESTS ####

# test the fetching of client configs
CONNECT_STRING=$(cat $FM_CFG_DIR/client-connect.json)
rm $FM_CFG_DIR/client.json
$FM_MINT_CLIENT join-federation "$CONNECT_STRING"

# # reissue
# NOTES=$($FM_MINT_CLIENT spend '42000msat' | jq -e -r '.note')
# [[ $($FM_MINT_CLIENT info | jq -e -r '.total_amount') = "9958000" ]]
# $FM_MINT_CLIENT validate $NOTES
# $FM_MINT_CLIENT reissue $NOTES
# $FM_MINT_CLIENT fetch

# # peg out
# PEG_OUT_ADDR="$($FM_BTC_CLIENT getnewaddress)"
# $FM_MINT_CLIENT peg-out $PEG_OUT_ADDR 500
# sleep 5 # FIXME wait for tx to be included
# await_fedimint_block_sync
# until [ "$($FM_BTC_CLIENT getreceivedbyaddress $PEG_OUT_ADDR 0)" == "0.00000500" ]; do
#   sleep $POLL_INTERVAL
# done
# mine_blocks 10
# RECEIVED=$($FM_BTC_CLIENT getreceivedbyaddress $PEG_OUT_ADDR)
# [[ "$RECEIVED" = "0.00000500" ]]

# # fedimint-cli pays CLN via LND gateaway
# switch_to_lnd_gateway
# INVOICE="$($FM_CLN invoice 100000 lnd-gw-to-cln test 1m | jq -e -r '.bolt11')"
# await_cln_block_processing
# $FM_MINT_CLIENT ln-pay $INVOICE
# # Check that gateway has received the ecash notes from the user payment
# # 100,000 sats + 100 sats without processing fee
# # FIXME ^^ comment isn't right
# FED_ID="$(get_federation_id)"
# LN_GATEWAY_BALANCE="$($FM_GWCLI_LND balance $FED_ID | jq -e -r '.balance_msat')"
# [[ "$LN_GATEWAY_BALANCE" = "100100000" ]]
# INVOICE_RESULT="$($FM_CLN waitinvoice lnd-gw-to-cln)"
# INVOICE_STATUS="$(echo $INVOICE_RESULT | jq -e -r '.status')"
# [[ "$INVOICE_STATUS" = "paid" ]]

# # fedimint-cli pays LND via CLN gateaway
# switch_to_cln_gateway
# ADD_INVOICE="$($FM_LND addinvoice --amt_msat 100000)"
# INVOICE="$(echo $ADD_INVOICE| jq -e -r '.payment_request')"
# PAYMENT_HASH="$(echo $ADD_INVOICE| jq -e -r '.r_hash')"
# await_cln_block_processing
# $FM_MINT_CLIENT ln-pay $INVOICE
# # Check that gateway has received the ecash notes from the user payment
# # 100,000 sats + 100 sats without processing fee
# # FIXME ^^ comment isnt' right
# FED_ID="$(get_federation_id)"
# LN_GATEWAY_BALANCE="$($FM_GWCLI_CLN balance $FED_ID | jq -e -r '.balance_msat')"
# [[ "$LN_GATEWAY_BALANCE" = "100100000" ]]
# INVOICE_STATUS="$($FM_LND lookupinvoice $PAYMENT_HASH | jq -e -r '.state')"
# [[ "$INVOICE_STATUS" = "SETTLED" ]]

# # LND pays CLN directly
# INVOICE="$($FM_CLN invoice 42000 lnd-to-cln test 1m | jq -e -r '.bolt11')"
# $FM_LND payinvoice --force "$INVOICE"
# INVOICE_STATUS="$($FM_CLN waitinvoice lnd-to-cln | jq -e -r '.status')"
# [[ "$INVOICE_STATUS" = "paid" ]]

# # CLN pays LND directly
# ADD_INVOICE="$($FM_LND addinvoice --amt_msat 42000)"
# INVOICE="$(echo $ADD_INVOICE| jq -e -r '.payment_request')"
# PAYMENT_HASH="$(echo $ADD_INVOICE| jq -e -r '.r_hash')"
# $FM_CLN pay "$INVOICE"
# INVOICE_STATUS="$($FM_LND lookupinvoice $PAYMENT_HASH | jq -e -r '.state')"
# [[ "$INVOICE_STATUS" = "SETTLED" ]]

# CLN pays user via LND gateway
# switch_to_lnd_gateway
# echo "paying\n\n\n\n"
# INVOICE="$($FM_MINT_CLIENT ln-invoice '100000msat' 'integration test' | jq -e -r '.invoice')"
# INVOICE_RESULT=$($FM_CLN pay $INVOICE)
# INVOICE_STATUS="$(echo $INVOICE_RESULT | jq -e -r '.status')"
# [[ "$INVOICE_STATUS" = "complete" ]]
