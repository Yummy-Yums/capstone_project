use std::error::Error;
pub use bdk_wallet::chain::spk_client::{FullScanRequest, FullScanResponse};
pub use bdk_wallet::chain::{BlockId, CheckPoint, ConfirmationBlockTime, TxUpdate};
pub use bdk_wallet::KeychainKind;
use bitcoin::Transaction;

pub trait ChainBackend {

    type Error: Error + Send + Sync + 'static;

    fn full_scan(
        &self,
        request: FullScanRequest<KeychainKind>,
        stop_gap: usize
    ) -> Result<FullScanResponse<KeychainKind>, Self::Error>;

    fn broadcast(&self, tx: &Transaction) -> Result<(), Self::Error>;
}