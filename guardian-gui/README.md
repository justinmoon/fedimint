### Running it

Run a federation without doing DKG:

```
./scripts/run-ui.sh
```

Run the frontend in 2 different terminal windows representing "leader" and "follower":

```
# connected to peer 0
PORT=3000 REACT_APP_FM_CONFIG_API=ws://127.0.0.1:18174 yarn start

# connected to peer 1
PORT=3001 REACT_APP_FM_CONFIG_API=ws://127.0.0.1:18184 yarn start
```