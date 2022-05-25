mkdir -p it/bitcoin
bitcoind -regtest -fallbackfee=0.00001 -txindex -server -rpcuser=bitcoin -rpcpassword=bitcoin -datadir=it/bitcoin &
BTC_CLIENT="bitcoin-cli -regtest -rpcuser=bitcoin -rpcpassword=bitcoin"
until [ "$($BTC_CLIENT getblockchaininfo | jq -r '.chain')" == "regtest" ]; do
  sleep $POLL_INTERVAL
done
lightningd --network regtest --bitcoin-rpcuser=bitcoin --bitcoin-rpcpassword=bitcoin --lightning-dir=it/ln1 --addr=127.0.0.1:9000 &
lightningd --network regtest --bitcoin-rpcuser=bitcoin --bitcoin-rpcpassword=bitcoin --lightning-dir=it/ln2 --addr=127.0.0.1:9001 &
