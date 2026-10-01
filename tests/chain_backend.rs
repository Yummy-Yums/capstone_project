use std::default::Default;
use std::sync::Mutex;
use bitcoin::Transaction;
use capstone_project::backend::{
    ChainBackend, FullScanRequest, FullScanResponse, KeychainKind, TxUpdate,
};
use capstone_project::{Amount, FeeRate, Mnemonic, Network, Recipient, Wallet};

const TEST_MNEMONIC: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";


struct MockBackend {
    amount_sats: u64,
    broadcasted: Mutex<Vec<bitcoin::Transaction>>,
}

#[derive(Debug, thiserror::Error)]
#[error("mock backend failure")]
struct MockBackendError;

impl ChainBackend for MockBackend {
    type Error = MockBackendError;

    fn full_scan(
        &self,
        mut request: FullScanRequest<KeychainKind>,
        _stop_gap: usize
    ) -> Result<FullScanResponse<KeychainKind>, Self::Error> {
        let (index, spk) = request
            .iter_spks(KeychainKind::External)
            .next()
            .expect("a fresh wallet always requests at least its first external spk");

        let funding_tx = bitcoin::Transaction {
            version: bitcoin::transaction::Version::TWO,
            lock_time: bitcoin::absolute::LockTime::ZERO,
            input: vec![bitcoin::TxIn {
                previous_output: bitcoin::OutPoint {
                    txid: <bitcoin::Txid as bitcoin::hashes::Hash>::from_byte_array([0x01; 32]),
                    vout: 0
                },
                ..Default::default()
            }],
            output: vec![bitcoin::TxOut {
                value: Amount::from_sat(self.amount_sats),
                script_pubkey: spk
            }],
        };

        let txid = funding_tx.compute_txid();

        let mut tx_update = TxUpdate::default();
        tx_update.txs
            .push(std::sync::Arc::new(funding_tx));
        tx_update.seen_ats
            .insert(txid, 1_700_000_000);

        let mut last_active_indices = std::collections::BTreeMap::new();
        last_active_indices.insert(KeychainKind::External, index);

        Ok(FullScanResponse {
            tx_update,
            last_active_indices,
            chain_update: None,
        })
    }

    fn broadcast(&self, tx: &Transaction) -> Result<(), Self::Error> {
       self.broadcasted
           .lock()
           .unwrap()
           .push(tx.clone());
        Ok(())
    }
}

fn test_wallet() -> Wallet {
    let mnemonic = Mnemonic::parse(TEST_MNEMONIC)
        .expect("valid test mnemonic");
    Wallet::from_mnemonic(&mnemonic, None, Network::Regtest)
        .expect("valid mnemonic wallet")
}

#[test]
fn sync_pulls_a_payment_in_through_the_backend(){
    let mut wallet = test_wallet();
    assert_eq!(wallet.balance().total(),  Amount::ZERO);

    let backend = MockBackend {
        amount_sats: 42_000,
        broadcasted: Mutex::new(Vec::new()),
    };

    wallet.sync(&backend)
        .expect("mock backend never errors");

    assert_eq!(wallet.balance().untrusted_pending,  Amount::from_sat(42_000));
    assert_eq!(wallet.balance().total(),  Amount::from_sat(42_000));

    let utxos = wallet.list_utxos();

    assert_eq!(utxos.len(), 1);
    assert_eq!(utxos[0].txout.value, Amount::from_sat(42_000));
    assert!(!utxos[0].confirmation.is_confirmed())

}

#[test]
fn broadcast_forwards_the_transaction_to_the_backend(){
    let wallet = test_wallet();
    let backend = MockBackend {
        amount_sats: 0,
        broadcasted: Mutex::new(Vec::new()),
    };

    let dummy_tx = bitcoin::Transaction {
        version: bitcoin::transaction::Version::TWO,
        lock_time: bitcoin::absolute::LockTime::ZERO,
        input: vec![],
        output: vec![],
    };

    wallet.broadcast(&backend, &dummy_tx)
        .expect("broadcast failed");

    let seen = backend.broadcasted
        .lock().unwrap();
    assert_eq!(seen.len(), 1);
}

#[test]
fn sync_is_idempotent_given_the_same_backend_report(){
    let mut wallet = test_wallet();
    let backend = MockBackend {
        amount_sats: 1_000,
        broadcasted: Mutex::new(Vec::new()),
    };

    wallet.sync(&backend).unwrap();
    let balance_after_first = wallet.balance();

    wallet.sync(&backend).unwrap();
    let balance_after_second = wallet.balance();

    assert_eq!(balance_after_first, balance_after_second);
    assert_eq!(wallet.list_utxos().len(), 1, "re-scanning must not duplicate the utxo");

}

#[test]
fn funds_synced_through_the_backend_are_spendable() {

    let mut wallet = test_wallet();
    let backed = MockBackend {
        amount_sats: 100_000,
        broadcasted: Mutex::new(Vec::new()),
    };

    wallet.sync(&backed).unwrap();

    let change_address = wallet.peek_address(capstone_project::Keychain::Internal, 0)
        .address;
    let recipients = [Recipient::new(change_address, Amount::from_sat(10_000))];

    let mut psbt = wallet
        .build_tx(&recipients, FeeRate::from_sat_per_vb(1).unwrap())
        .expect("synced funds must be spendable");

    let fully_signed = wallet.sign(&mut psbt)
        .expect("signing must succeed");

    assert!(fully_signed);

    let tx = wallet.finalize(&mut psbt)
        .expect("psbt is fully signed");
    assert_eq!(tx.input.len(), 1, "spends the one utxo the backend reported");

}