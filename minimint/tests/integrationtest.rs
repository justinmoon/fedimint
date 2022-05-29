use assert_matches::assert_matches;
use bitcoin::schnorr::PublicKey;
use bitcoin::Amount;
use fixture::fixtures;
use fixture::{rng, sats};
use minimint::consensus::ConsensusItem;
use minimint_wallet::WalletConsensusItem::PegOutSignature;
use std::ops::Sub;

mod fixture;

#[tokio::test(flavor = "multi_thread")]
async fn peg_in_and_peg_out_with_fees() {
    let (fed, user, bitcoin, _, _) = fixtures(2, 0, &[sats(100), sats(1000)]).await;
    let peg_in_address = user.client.get_new_pegin_address(rng());

    let (proof, tx) = bitcoin.send_and_mine_block(&peg_in_address, Amount::from_sat(5000));
    bitcoin.mine_blocks(fed.wallet.finalty_delay as u64);
    fed.run_consensus_epochs(2).await;

    user.client.peg_in(proof, tx, rng()).await.unwrap();
    fed.run_consensus_epochs(4).await; // peg in epoch + partial sigs epoch

    user.client.fetch_all_coins().await.unwrap();
    assert_eq!(
        user.total_coins(),
        sats(5000).sub(fed.fee_consensus.fee_peg_in_abs)
    );
    let peg_out_address = bitcoin.get_new_address();
    user.client
        .peg_out(Amount::from_sat(1000), peg_out_address.clone(), rng())
        .await
        .unwrap();
    fed.run_consensus_epochs(2).await;

    bitcoin.mine_blocks(minimint_wallet::MIN_PEG_OUT_URGENCY as u64 + 1);
    fed.run_consensus_epochs(5).await; // block height epoch + 2 peg out signing epochs
    assert_matches!(
        fed.last_consensus().first(),
        Some(ConsensusItem::Wallet(PegOutSignature(_)))
    );

    fed.broadcast_transactions().await;
    assert_eq!(
        bitcoin.mine_block_and_get_received(&peg_out_address),
        sats(1000)
    );
    assert_eq!(
        user.total_coins(),
        sats(4000)
            .sub(fed.fee_consensus.fee_peg_in_abs)
            .sub(fed.fee_consensus.fee_peg_out_abs)
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn peg_out_with_p2pkh() {
    // FIXME Hash of peg-out transaction doesn't match in wallet and fed code, causing InvalidSignature error
    let (fed, user, _, _, _) = fixtures(2, 0, &[sats(100), sats(1000)]).await;
    fed.mint_coins_for_user(&user, sats(4500)).await;

    let ctx = bitcoin::secp256k1::Secp256k1::new();
    let (_, public_key) = ctx.generate_keypair(&mut rng());
    let peg_out_address = bitcoin::Address::p2pkh(
        &bitcoin::ecdsa::PublicKey::new(public_key),
        bitcoin::Network::Regtest,
    );
    user.client
        .peg_out(Amount::from_sat(1000), peg_out_address.clone(), rng())
        .await
        .unwrap();
    fed.run_consensus_epochs(2).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn minted_coins_can_be_exchanged_between_users() {
    let (fed, user_send, _, _, _) = fixtures(2, 0, &[sats(100), sats(1000)]).await;
    let user_receive = user_send.new_client(&[0]);

    fed.mint_coins_for_user(&user_send, sats(5000)).await;
    assert_eq!(user_send.total_coins(), sats(5000));
    assert_eq!(user_receive.total_coins(), sats(0));

    let coins = user_send.client.select_and_spend_coins(sats(3000)).unwrap();
    let outpoint = user_receive.client.reissue(coins, rng()).await.unwrap();
    fed.run_consensus_epochs(4).await; // process transaction + sign new coins

    user_receive.client.fetch_coins(outpoint).await.unwrap();
    assert_eq!(user_send.total_coins(), sats(2000));
    assert_eq!(user_receive.total_coins(), sats(3000));
}

#[tokio::test(flavor = "multi_thread")]
async fn minted_coins_cannot_double_spent_with_different_nodes() {
    let (fed, user1, _, _, _) = fixtures(2, 0, &[sats(100), sats(1000)]).await;
    fed.mint_coins_for_user(&user1, sats(5000)).await;
    let coins = user1.client.select_and_spend_coins(sats(2000)).unwrap();

    let (user2, user3) = (user1.new_client(&[0]), user1.new_client(&[1]));
    let out2 = user2.client.reissue(coins.clone(), rng()).await.unwrap();
    let out3 = user3.client.reissue(coins, rng()).await.unwrap();
    fed.run_consensus_epochs(4).await; // process transaction + sign new coins

    // FIXME is this the correct behavior, that the first one goes through?
    assert!(user2.client.fetch_coins(out2).await.is_ok());
    assert!(user3.client.fetch_coins(out3).await.is_err());
    assert_eq!(user2.total_coins(), sats(2000));
    assert_eq!(user3.total_coins(), sats(0));
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
// FIXME should be able to fetch change coins fails with NotEnoughCoins
// https://github.com/fedimint/minimint/issues/11
async fn minted_coins_in_wallet_can_be_split_into_change() {
    let (fed, user, _, _, _) = fixtures(2, 0, &[sats(100), sats(1000)]).await;
    fed.mint_coins_for_user(&user, sats(2100)).await;
    assert_eq!(user.coin_amounts(), vec![sats(100), sats(1000), sats(1000)]);

    user.client.select_and_spend_coins(sats(1900)).unwrap();
    fed.run_consensus_epochs(4).await; // process transaction + sign new coins
    assert_eq!(user.coin_amounts(), vec![sats(100), sats(100)]);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn minted_coins_can_be_created_with_different_max_evil_thresholds() {
    // FIXME Fails because of InvalidSignature error
    let (fed, user, _, _, _) = fixtures(5, 0, &[sats(100), sats(1000)]).await;
    fed.mint_coins_for_user(&user, sats(1000)).await;
    assert_eq!(user.total_coins(), sats(1000));

    // FIXME Fails because HBBFT consensus hangs without all nodes
    let (fed, user, _, _, _) = fixtures(3, 1, &[sats(100), sats(1000)]).await;
    fed.subset_peers(&[0, 1])
        .mint_coins_for_user(&user, sats(2000))
        .await;
    assert_eq!(user.total_coins(), sats(2000));
}

#[tokio::test(flavor = "multi_thread")]
async fn lightning_gateway_pays_invoice() {
    let (fed, user, _, gateway, lightning) = fixtures(2, 0, &[sats(10), sats(1000)]).await;
    let invoice = lightning.invoice(sats(1000));

    fed.mint_coins_for_user(&user, sats(1010)).await; // 1% LN fee
    let contract_id = user
        .client
        .fund_outgoing_ln_contract(&gateway.keys, invoice, rng())
        .await
        .unwrap();
    fed.run_consensus_epochs(2).await; // send coins to LN contract

    let contract_account = user.client.wait_contract(contract_id).await.unwrap();
    assert_eq!(contract_account.amount, sats(1010));
    gateway
        .server
        .pay_invoice(contract_id, rng())
        .await
        .unwrap();
    fed.run_consensus_epochs(4).await; // contract to mint coins, sign coins

    gateway.server.await_contract_claimed(contract_id).await;
    assert_eq!(user.total_coins(), sats(0));
    assert_eq!(gateway.user.client.coins().amount(), sats(1010));
    assert_eq!(lightning.amount_sent(), sats(1000));
}

#[tokio::test(flavor = "multi_thread")]
async fn receive_lightning_payment_via_gateway() {
    let amount = sats(2000);
    let (fed, user, _, gateway, _) = fixtures(2, 0, &[sats(10), sats(1000)]).await;
    fed.mint_coins_for_user(&gateway.user, amount).await;
    assert_eq!(user.total_coins(), sats(0));
    assert_eq!(gateway.user.total_coins(), amount);

    // Create invoice and offer in the federation (should this block in the future?)
    let (keypair, invoice) = user
        .client
        .create_invoice_and_offer(&gateway.keys, amount, rng())
        .await
        .unwrap();

    // Run 2 epochs to get the offer to show up p
    fed.run_consensus_epochs(2).await;

    let offers = user.client.get_offers().await.unwrap();
    assert_eq!(&offers[0].hash, invoice.payment_hash());

    // Gateway deposits ecash to trigger preimage decryption by the federation
    let (txid, contract_id) = gateway
        .server
        .buy_preimage_offer(invoice.payment_hash(), &amount, rng())
        .await
        .unwrap();

    fed.run_consensus_epochs(5).await; // announce preimage sale

    // Gateway receives decrypted preimage and it matches the invoice payment hash
    let preimage = gateway
        .server
        .await_preimage_decryption(txid)
        .await
        .unwrap();

    // Check that the preimage matches user pubkey
    let secp = bitcoin::secp256k1::Secp256k1::new();
    assert_eq!(PublicKey::from_keypair(&secp, &keypair), preimage.0);

    // User claims their ecash
    user.client
        .claim_incoming_contract(contract_id, keypair, rng())
        .await
        .unwrap();
    fed.run_consensus_epochs(4).await; // FIXME: why does this need 4 rounds?

    // User fetches their coins
    user.client.fetch_all_coins().await.unwrap();
    fed.run_consensus_epochs(2).await;

    // Ecash tokens have been transferred from gateway to user
    assert_eq!(gateway.user.client.coins().amount(), sats(0));
    assert_eq!(user.total_coins(), amount);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn receive_lightning_payment_via_gateway_invalid_preimage() {
    unimplemented!();
}

#[tokio::test(flavor = "multi_thread")]
async fn lightning_gateway_cannot_claim_invalid_preimage() {
    let (fed, user, _, gateway, lightning) = fixtures(2, 0, &[sats(10), sats(1000)]).await;
    let invoice = lightning.invoice(sats(1000));

    fed.mint_coins_for_user(&user, sats(1010)).await; // 1% LN fee
    let contract_id = user
        .client
        .fund_outgoing_ln_contract(&gateway.keys, invoice, rng())
        .await
        .unwrap();
    fed.run_consensus_epochs(2).await; // send coins to LN contract

    let bad_preimage: [u8; 32] = rand::random();
    let response = gateway
        .client
        .claim_outgoing_contract(contract_id, bad_preimage, rng())
        .await;
    assert!(response.is_err());

    fed.run_consensus_epochs(2).await; // if valid would create contract to mint coins
    assert!(fed.last_consensus().is_empty())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn lightning_gateway_can_abort_payment_to_return_user_funds() {
    let (fed, user, _, gateway, lightning) = fixtures(2, 0, &[sats(10), sats(1000)]).await;
    let invoice = lightning.invoice(sats(1000));

    fed.mint_coins_for_user(&user, sats(1010)).await; // 1% LN fee
    let contract_id = user
        .client
        .fund_outgoing_ln_contract(&gateway.keys, invoice, rng())
        .await
        .unwrap();
    fed.run_consensus_epochs(2).await; // send coins to LN contract

    // FIXME should return funds to user
    gateway.client.abort_outgoing_payment(contract_id);
    fed.run_consensus_epochs(2).await;
    assert_eq!(user.total_coins(), sats(1010));
}
