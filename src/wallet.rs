use crate::backend::ChainBackend;
use crate::error::WalletError;
use crate::types::{AddressInfo, Balance, ConfirmationStatus, Keychain};
use crate::types::{Recipient, Utxo};
use bdk_wallet::template::Bip84;
use bdk_wallet::{KeychainKind, SignOptions, Wallet as BdkWallet};
use bip39::Mnemonic;
use bitcoin::{FeeRate, Network, Psbt, Transaction};
use miniscript::{Descriptor, DescriptorPublicKey};

pub const DEFAULT_STOP_GAP: usize = 20;


#[derive(Debug)]
pub struct Wallet {
    inner: BdkWallet,
    stop_gap: usize
}

impl Wallet {

    pub fn from_mnemonic(
        mnemonic: &Mnemonic,
        passphrase: Option<&str>,
        network: Network,
    ) -> Result<Self, WalletError> {
        let keyed = (mnemonic.clone(), passphrase.map(str::to_string));
        let external = Bip84(keyed.clone(), KeychainKind::External);
        let internal = Bip84(keyed, KeychainKind::Internal);

        let inner = BdkWallet::create(external, internal)
            .network(network)
            .create_wallet_no_persist()
            .map_err(|e| WalletError::WalletCreation(format!("{}", e)))?;

        Ok(Self {
            inner,
            stop_gap: DEFAULT_STOP_GAP
        })
    }

    pub fn from_descriptor(
        external: &str,
        internal: Option<&str>,
        network: Network,
    ) -> Result<Self, WalletError> {
        validate_descriptor(external)?;
        if let Some(internal) = internal {
            validate_descriptor(internal)?;
        }

        let inner = match internal {
            Some(internal) => BdkWallet::create(external.to_string(), internal.to_string())
                .network(network)
                .create_wallet_no_persist(),
            None => BdkWallet::create_single(external.to_string())
                .network(network)
                .create_wallet_no_persist()
        }
            .map_err(|e| WalletError::WalletCreation(format!("{}", e)))?;

        Ok(Self {
            inner,
            stop_gap: DEFAULT_STOP_GAP
        })
    }

    pub fn network(&self) -> Network {
        self.inner.network()
    }

    pub fn new_address(&mut self) -> AddressInfo {
        let info = self.inner.reveal_next_address(KeychainKind::External);
        AddressInfo {
            index: info.index,
            address: info.address,
            keychain: Keychain::from_bdk(info.keychain)
        }
    }

    pub fn peek_address(&self, keychain: Keychain, index: u32) -> AddressInfo {
        let info = self.inner.peek_address(keychain.to_bdk(), index);
        AddressInfo {
            index: info.index,
            address: info.address,
            keychain: Keychain::from_bdk(info.keychain)
        }
    }

    pub fn balance(&self) -> Balance { Balance::from_bdk(self.inner.balance()) }

    pub fn list_utxos(&self) -> Vec<Utxo> {
        self.inner
            .list_unspent()
            .map(|utxo| Utxo {
                outpoint: utxo.outpoint,
                txout: utxo.txout,
                keychain: Keychain::from_bdk(utxo.keychain),
                is_spent: utxo.is_spent,
                confirmation: match utxo.chain_position {
                    bdk_wallet::chain::ChainPosition::Confirmed { anchor, .. } => {
                        ConfirmationStatus::Confirmed {
                            height: anchor.block_id.height
                        }
                    }
                    bdk_wallet::chain::ChainPosition::Unconfirmed { .. } => {
                        ConfirmationStatus::Unconfirmed
                    }
                },
            })
            .collect()
    }

    pub fn build_tx(
        &mut self,
        recipients: &[Recipient],
        fee_rate: FeeRate,
    ) -> Result<Psbt, WalletError>{
        if recipients.is_empty() {
            return Err(WalletError::NoRecipients);
        }

        let mut builder = self.inner.build_tx();
        for recipient in recipients {
            builder.add_recipient(recipient.address.script_pubkey(), recipient.amount);
        }

        builder.fee_rate(fee_rate);

        builder.finish()
            .map_err(|e| WalletError::BuildTx(format!("{:?}", e)))
    }

    pub fn sign(&self, psbt: &mut Psbt) -> Result<bool, WalletError> {
        self.inner
            .sign(psbt, SignOptions::default())
            .map_err(|e| WalletError::Sign(e.to_string()))
    }

    pub fn finalize(&self, psbt: &mut Psbt) -> Result<Transaction, WalletError> {
        let finalized = self
            .inner
            .finalize_psbt(psbt, SignOptions::default())
            .map_err(|e| WalletError::Sign(e.to_string()))?;

        if !finalized {
            let incomplete = psbt
                .inputs
                .iter()
                .filter(|input| {
                    input.final_script_sig.is_none() &&
                        input.final_script_witness.is_none()
                })
                .count();

            return Err(WalletError::IncompletePsbt(incomplete));
        }

        psbt.clone()
            .extract_tx()
            .map_err(|e| WalletError::ExtractTx(e.to_string()))
    }

    pub fn sync<B: ChainBackend>(&mut self, backend: &B) -> Result<(), WalletError> {
        let request = self.inner.start_full_scan().build();
        let response = backend
            .full_scan(request, self.stop_gap)
            .map_err(WalletError::backend)?;

        self.inner
            .apply_update(response)
            .map_err(|e| WalletError::ApplyUpdate(e.to_string()))
    }

    pub fn broadcast<B: ChainBackend>(&self, backend: &B, tx: &Transaction) -> Result<(), WalletError> {
        backend.broadcast(tx)
            .map_err(WalletError::backend)
    }
}

fn validate_descriptor(descriptor: &str) -> Result<(), WalletError> {
    match descriptor.parse::<Descriptor<DescriptorPublicKey>>(){
        Ok(_) => Ok(()),
        Err(_) if descriptor.contains("prv") => Ok(()),
        Err(e) => Err(WalletError::InvalidDescriptor(e.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bdk_wallet::test_utils::{insert_anchor, insert_checkpoint, insert_tx, new_tx};
    use bdk_wallet::chain::{BlockId, ConfirmationBlockTime};
    use bitcoin::hashes::Hash;
    use bitcoin::{Address, Amount, BlockHash, TxIn, TxOut};
    use std::str::FromStr;

    const TEST_MNEMONIC: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

    fn test_mnemonic() -> Mnemonic {
        Mnemonic::parse(TEST_MNEMONIC)
            .expect("Failed to parse test mnemonic")
    }

    #[test]
    fn mnemonic_wallet_derives_deterministic_addresses(){
        let mut a = Wallet::from_mnemonic(&test_mnemonic(), None, Network::Testnet).unwrap();
        let mut b = Wallet::from_mnemonic(&test_mnemonic(), None, Network::Testnet).unwrap();

        assert_eq!(a.new_address().address, b.new_address().address);
        assert_eq!(a.new_address().address, b.new_address().address);
    }

    #[test]
    fn none_and_empty_passphrase_are_equivalent_but_a_real_one_differs(){
        let mut with_none = Wallet::from_mnemonic(&test_mnemonic(), None, Network::Testnet).unwrap();
        let mut with_empty = Wallet::from_mnemonic(&test_mnemonic(), Some(""), Network::Testnet).unwrap();
        let mut with_word = Wallet::from_mnemonic(&test_mnemonic(), Some("river"), Network::Testnet).unwrap();

        let a = with_none.new_address().address;
        let b = with_empty.new_address().address;
        let c = with_word.new_address().address;

        assert_eq!(a, b, "None and Some(\"\") both mean \"no passphrase\"");
        assert_ne!(b, c, "a real passphrade must produce a different wallet");
    }

    #[test]
    fn new_address_advances_index_and_keychain(){
        let mut wallet = Wallet::from_mnemonic(&test_mnemonic(), None, Network::Testnet).unwrap();

        let first = wallet.new_address();
        let second = wallet.new_address();

        assert_eq!(first.index, 0);
        assert_eq!(second.index, 1);
        assert_ne!(first.address, second.address);
        assert_eq!(first.keychain, Keychain::External);
        assert_eq!(second.keychain, Keychain::External);
    }

    #[test]
    fn peek_address_does_not_advance_the_index(){
        let wallet = Wallet::from_mnemonic(&test_mnemonic(), None, Network::Signet).unwrap();

        let peaked_twice_a = wallet.peek_address(Keychain::External, 5);
        let peaked_twice_b = wallet.peek_address(Keychain::External, 5);

        assert_eq!(peaked_twice_a.address, peaked_twice_b.address);
    }

    #[test]
    fn descriptor_wallet_matches_known_vector(){

        let external = "wpkh([c55b303f/84'/1'/0']tpubDC2Qwo2TFsaNC4ju8nrUJ9mqVT3eSgdmy1yPqhgkjwmke3PRXutNGRYAUo6RCHTcVQaDR3ohNU9we59brGHuEKPvH1ags2nevW5opEE9Z5Q/0/*)";
        let mut wallet = Wallet::from_descriptor(external, None, Network::Testnet).unwrap();

        assert_eq!(
            wallet.new_address().address.to_string(),
            "tb1qedg9fdlf8cnnqfd5mks6uz5w4kgpk2pr6y4qc7"
        );

    }

    #[test]
    fn garbage_descriptor_is_rejected_before_wallet_construction(){
        let err = Wallet::from_descriptor("not a descriptor", None, Network::Testnet)
            .expect_err("garbage input must fail");
        assert!(matches!(err, WalletError::InvalidDescriptor(_)))
    }

    #[test]
    fn fresh_wallet_has_zero_balance_and_no_utxos(){
        let wallet = Wallet::from_mnemonic(&test_mnemonic(), None, Network::Regtest).unwrap();
        assert_eq!(wallet.balance(), Balance::default());
        assert!(wallet.list_utxos().is_empty());
    }

    #[test]
    fn build_tx_with_no_recipients_is_rejected(){
        let mut wallet = Wallet::from_mnemonic(&test_mnemonic(), None, Network::Regtest).unwrap();
        let err = wallet
            .build_tx(&[], FeeRate::from_sat_per_vb(1).unwrap())
            .expect_err("Failed to build tx");
        assert!(matches!(err, WalletError::NoRecipients))
    }

    fn fund_wallet(wallet: &mut Wallet){
        let receive_address = wallet.peek_address(Keychain::External, 0).address;
        let send_to_address = Address::from_str("bcrt1q3qtze4ys45tgdvguj66zrk4fu6hq3a3v9pfly5")
            .unwrap()
            .require_network(Network::Regtest)
            .unwrap();

        let tx0 = bitcoin::Transaction {
            output: vec![TxOut {
                value: Amount::from_sat(76_000),
                script_pubkey: receive_address.script_pubkey(),
            }],
            ..new_tx(0)
        };

        let tx1 = bitcoin::Transaction {
            input: vec![TxIn {
                previous_output: bitcoin::OutPoint {
                    txid: tx0.compute_txid(),
                    vout: 0,
                },
                ..Default::default()
            }],
            output: vec![
                TxOut {
                    value: Amount::from_sat(50_000),
                    script_pubkey: receive_address.script_pubkey(),
                },
                TxOut {
                    value: Amount::from_sat(25_000),
                    script_pubkey: send_to_address.script_pubkey(),
                },
            ],
            ..new_tx(0)
        };

        for height in [42, 1_000, 2_000]{
            insert_checkpoint(
                &mut wallet.inner,
                BlockId {
                    height,
                    hash: BlockHash::all_zeros()
                },
            );
        }

        insert_tx(&mut wallet.inner, tx0.clone());
        insert_anchor(
            &mut wallet.inner,
            tx0.compute_txid(),
            ConfirmationBlockTime {
                block_id: BlockId {
                    height: 1_000,
                    hash: BlockHash::all_zeros(),
                },
                confirmation_time: 100
            }
        );

        insert_tx(&mut wallet.inner, tx1.clone());
        insert_anchor(
            &mut wallet.inner,
            tx1.compute_txid(),
            ConfirmationBlockTime {
                block_id: BlockId {
                    height: 2_000,
                    hash: BlockHash::all_zeros(),
                },
                confirmation_time: 200
            },
        );
    }

    #[test]
    fn funded_wallet_reports_correct_balance_and_utxos(){
        let mut wallet = Wallet::from_mnemonic(&test_mnemonic(), None, Network::Regtest).unwrap();
        fund_wallet(&mut wallet);

        let balance = wallet.balance();
        assert_eq!(balance.confirmed, Amount::from_sat(50_000));
        assert_eq!(balance.total(), Amount::from_sat(50_000));

        let utxos = wallet.list_utxos();
        assert_eq!(utxos.len(), 1);
        assert_eq!(utxos[0].txout.value, Amount::from_sat(50_000));
        assert!(utxos[0].confirmation.is_confirmed());
        assert!(!utxos[0].is_spent);
    }

    #[test]
    fn build_sign_and_finalize_a_real_transaction(){
        let mut wallet = Wallet::from_mnemonic(&test_mnemonic(), None, Network::Regtest).unwrap();
        fund_wallet(&mut wallet);

        let destination = Address::from_str("bcrt1q3qtze4ys45tgdvguj66zrk4fu6hq3a3v9pfly5")
            .unwrap()
            .require_network(Network::Regtest)
            .unwrap();

        let recipients = [Recipient::new(destination, Amount::from_sat(10_000))];

        let mut psbt = wallet
            .build_tx(&recipients, FeeRate::from_sat_per_vb(2).unwrap())
            .expect("enough confirmed funds to build a tx");

        let fully_signed = wallet.sign(&mut psbt).expect("signing must succeed");
        assert!(fully_signed, "wallet holds every input's key, so signing should fully finalize");

        let tx = wallet.finalize(&mut psbt)
            .expect("psbt is fully signed");

        assert!(tx.output.len() >= 1);
        let res = tx.output
            .iter()
            .any(|out| out.script_pubkey == recipients[0].address.script_pubkey() && out.value == recipients[0].amount);
        assert!(res);

        assert_eq!(tx.input.len(), 1);

        let fee = wallet.inner.calculate_fee(&tx)
            .expect("fee must be calculable");

        assert!(fee.to_sat() > 0, "a real fee must have been paid")

    }

    #[test]
    fn finalize_without_signing_reports_incomplete_psbt(){
        let mut wallet = Wallet::from_mnemonic(&test_mnemonic(), None, Network::Regtest).unwrap();
        fund_wallet(&mut wallet);

        let destination = Address::from_str("bcrt1q3qtze4ys45tgdvguj66zrk4fu6hq3a3v9pfly5")
            .unwrap()
            .require_network(Network::Regtest)
            .unwrap();

        let recipients = [Recipient::new(destination, Amount::from_sat(10_000))];

        let mut psbt = wallet
            .build_tx(&recipients, FeeRate::from_sat_per_vb(2).unwrap())
            .unwrap();

        let err = wallet.finalize(&mut psbt)
            .expect_err("unsigned psbt cannot finalize");
        assert!(matches!(err, WalletError::IncompletePsbt(1)))
    }
}