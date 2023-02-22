#!/usr/bin/env bash
# Generates the configs and starts the federation nodes

echo "Staring Federation..."
set -euxo pipefail

# Start the federation members inside the temporary directory
for ((ID=START_SERVER; ID<END_SERVER; ID++)); do
  echo "starting mint $ID"
  export FM_PASSWORD="pass$ID"
  ( ($FM_BIN_DIR/fedimintd $FM_CFG_DIR/server-$ID 2>&1 & echo $! >&3 ) 3>>$FM_PID_FILE | sed -e "s/^/mint $ID: /" ) &
done

