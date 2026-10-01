pub mod backend;
pub mod error;
pub mod types;
mod wallet;

pub use backend::ChainBackend;
pub use error::WalletError;
pub use types::{AddressInfo, Balance, ConfirmationStatus, Keychain, Recipient, Utxo};
pub use wallet::{Wallet, DEFAULT_STOP_GAP};

pub use bitcoin::{Address, Amount, FeeRate, Network, Psbt, Transaction};

pub use bip39::{Language, Mnemonic};
