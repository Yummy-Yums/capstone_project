mod common;

use std::error::Error;

use common::DemoChain;
use capstone_project::{
    Amount, ConfirmationStatus, FeeRate, Keychain, Mnemonic, Network, Recipient, Wallet,
};

const OWN_MNEMONIC: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

const COUNTERPARTY_MNEMONIC: &str =
    "legal winner thank year wave sausage worth useful legal winner thank yellow";

fn main() -> Result<(), Box<dyn Error>> {

    let mnemonic = Mnemonic::parse(OWN_MNEMONIC)?;
    let mut wallet = Wallet::from_mnemonic(&mnemonic, None, Network::Signet)?;
    println!("wallet created for network {}", wallet.network());

    println!("\n== receive addresses ==");
    for _ in 0..3 {
        let info = wallet.new_address();
        println!("  #{:<2} {}", info.index, info.address);
    }

    let first = wallet.peek_address(Keychain::External, 0);
    println!(
        "  peeked (not revealed) #{}  {}",
        first.index, first.address
    );

    let chain = DemoChain::new();
    let receive_address = wallet.peek_address(Keychain::External, 0).address;

    chain.fund_pending(receive_address.script_pubkey(), Amount::from_sat(120_000));
    wallet.sync(&chain)?;

    println!("\n== after first sync: payment seen, not yet mined ==");
    print_balance(&wallet);
    print_utxos(&wallet);

    chain.confirm_pending();
    wallet.sync(&chain)?;

    println!("\n== after the payment confirmed ==");
    print_balance(&wallet);

    let counterparty = Wallet::from_mnemonic(
        &Mnemonic::parse(COUNTERPARTY_MNEMONIC)?,
        None,
        Network::Signet,
    )?;
    let destination = counterparty.peek_address(Keychain::External, 0).address;

    let recipients = [
        Recipient::new(destination.clone(), Amount::from_sat(30_000)),
        Recipient::new(destination, Amount::from_sat(10_000)),
    ];

    let utxos_before = wallet.list_utxos();
    let mut psbt = wallet.build_tx(&recipients, FeeRate::from_sat_per_vb(2).unwrap())?;

    println!("\n== built psbt ==");
    println!(
        "  serialized: {} bytes, {} input(s)",
        psbt.serialize().len(),
        psbt.unsigned_tx.input.len()
    );
    for (i, input) in psbt.unsigned_tx.input.iter().enumerate() {
        println!(
            "  input {i}: spends {}:{}",
            input.previous_output.txid, input.previous_output.vout
        );
    }

    let fully_signed = wallet.sign(&mut psbt)?;
    println!("\n== signing ==");
    println!("  fully signed: {fully_signed}");

    let tx = wallet.finalize(&mut psbt)?;

    let input_total: u64 = psbt
        .unsigned_tx
        .input
        .iter()
        .filter_map(|i| {
            utxos_before
                .iter()
                .find(|u| u.outpoint == i.previous_output)
                .map(|u| u.txout.value.to_sat())
        })
        .sum();
    let output_total: u64 = tx.output.iter().map(|o| o.value.to_sat()).sum();

    println!("\n== finalized transaction ==");
    println!("  txid:          {}", tx.compute_txid());
    println!(
        "  {} input(s), {} output(s)",
        tx.input.len(),
        tx.output.len()
    );
    println!("  in:            {input_total} sats");
    println!("  out:           {output_total} sats");
    println!("  fee:           {} sats", input_total - output_total);

    wallet.broadcast(&chain, &tx)?;
    println!("\n== immediately after broadcast ==");
    println!("  (nothing has changed locally yet -- state updates on `sync`)");
    print_balance(&wallet);

    wallet.sync(&chain)?;
    println!("\n== after re-sync: our transaction is in the mempool ==");
    print_balance(&wallet);
    print_utxos(&wallet);

    chain.confirm_pending();
    wallet.sync(&chain)?;
    println!("\n== after our transaction confirmed ==");
    print_balance(&wallet);

    println!(
        "\n{} transaction(s) went out through the chain backend",
        chain.broadcast_log().len()
    );
    Ok(())
}

fn print_balance(wallet: &Wallet) {
    let b = wallet.balance();
    println!("  confirmed         {:>9} sats", b.confirmed.to_sat());
    println!("  trusted pending   {:>9} sats", b.trusted_pending.to_sat());
    println!(
        "  untrusted pending {:>9} sats",
        b.untrusted_pending.to_sat()
    );
    println!("  immature          {:>9} sats", b.immature.to_sat());
    println!("  -- spendable:     {:>9} sats", b.spendable().to_sat());
    println!("     total:         {:>9} sats", b.total().to_sat());
}

fn print_utxos(wallet: &Wallet) {
    println!("  utxos:");
    for utxo in wallet.list_utxos() {
        let status = match utxo.confirmation {
            ConfirmationStatus::Confirmed { height } => format!("confirmed @ {height}"),
            ConfirmationStatus::Unconfirmed => "unconfirmed".to_string(),
        };
        println!(
            "    {}:{}  {:>9} sats  [{:?} keychain, {}]",
            utxo.outpoint.txid,
            utxo.outpoint.vout,
            utxo.txout.value.to_sat(),
            utxo.keychain,
            status,
        );
    }
}