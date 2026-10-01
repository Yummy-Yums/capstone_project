use bitcoin::{Address, Amount, OutPoint, TxOut};
use bdk_wallet::KeychainKind;


#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Keychain {
    External,
    Internal,
}

impl Keychain {
    pub(crate) fn from_bdk(kind: KeychainKind)  -> Self {
        match kind {
            KeychainKind::External => Keychain::External,
            KeychainKind::Internal => Keychain::Internal,
        }
    }

    pub(crate) fn to_bdk(self) -> KeychainKind {
        match self {
            Keychain::External => KeychainKind::External,
            Keychain::Internal => KeychainKind::Internal,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AddressInfo {
    pub index: u32,
    pub address: Address,
    pub keychain: Keychain,
}

impl std::fmt::Display for AddressInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}@{}", self.index, self.address)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Balance {
    pub immature: Amount,
    pub trusted_pending: Amount,
    pub untrusted_pending: Amount,
    pub confirmed: Amount,
}

impl Balance {
    pub fn spendable(&self) -> Amount {
        self.confirmed + self.trusted_pending
    }
    
    pub fn total(&self) -> Amount {
        self.confirmed + self.trusted_pending + self.untrusted_pending + self.immature
    }
    
    pub(crate) fn from_bdk(balance: bdk_wallet::chain::Balance) -> Self {
        Balance {
            immature: balance.immature,
            trusted_pending: balance.trusted_pending,
            untrusted_pending: balance.untrusted_pending,
            confirmed: balance.confirmed,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmationStatus {
    Confirmed { height: u32 },
    Unconfirmed,
}

impl ConfirmationStatus {
    pub fn is_confirmed(self) -> bool {
        matches!(self, ConfirmationStatus::Confirmed {..})
    }
}

#[derive(Debug, Clone)]
pub struct Utxo {
    pub outpoint: OutPoint,
    pub txout: TxOut,
    pub keychain: Keychain,
    pub is_spent: bool,
    pub confirmation: ConfirmationStatus,
}

#[derive(Debug, Clone)]
pub struct Recipient {
    pub address: Address,
    pub amount: Amount,
}

impl Recipient {
    pub fn new(address: Address, amount: Amount) -> Self {
        Self { address, amount }
    }
}

impl From<(Address, Amount)> for Recipient {
    fn from((address, amount): (Address, Amount)) -> Self {
        Self { address, amount }
    }
}