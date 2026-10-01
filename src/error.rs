use std::error::Error;

#[derive(Debug, thiserror::Error)]
pub enum WalletError {
    #[error("invalid mnemonic: {0}")]
    InvalidMnemonic(String),

    #[error("invalid descriptor: {0}")]
    InvalidDescriptor(String),

    #[error("failed to construct wallet: {0}")]
    WalletCreation(String),

    #[error("no recipients specified")]
    NoRecipients,

    #[error("failed to build transaction: {0}")]
    BuildTx(String),

    #[error("failed to sign transaction: {0}")]
    Sign(String),

    #[error("psbt is not fully signed; {0} input(s) could not be finalized")]
    IncompletePsbt(usize),

    #[error("failed to extract final transaction from psbt: {0}")]
    ExtractTx(String),

    #[error("failed to apply chain update: {0}")]
    ApplyUpdate(String),

    #[error("chain backend error: {0}")]
    Backend(#[source] Box<dyn std::error::Error + Send + Sync + 'static>),
}

impl WalletError {
    pub fn backend<E>(err: E) -> Self
    where
        E: Error + Send + Sync + 'static, {
        WalletError::Backend(Box::new(err))
    }
}