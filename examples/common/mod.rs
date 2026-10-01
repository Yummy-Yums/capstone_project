#![allow(dead_code)]

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use bitcoin::hashes::sha256d;
use bitcoin::hashes::Hash;
use bitcoin::{
    absolute::LockTime, transaction::Version, Amount, BlockHash, OutPoint, ScriptBuf, Sequence,
    Transaction, TxIn, TxOut, Txid, Witness,
};
use capstone_project::backend::{
    BlockId, ChainBackend, CheckPoint, ConfirmationBlockTime, FullScanRequest, FullScanResponse,
    KeychainKind, TxUpdate,
};

/// Height of the chain tip a fresh [`DemoChain`] starts at.
const START_HEIGHT: u32 = 200;

#[derive(Debug, thiserror::Error)]
#[error("the demo chain cannot do that")]
pub struct DemoChainError;

pub struct DemoChain {
    tip_height: Mutex<u32>,
    funds: Mutex<HashMap<ScriptBuf, Amount>>,
    pending_funds: Mutex<HashMap<ScriptBuf, Amount>>,
    broadcast_log: Mutex<Vec<Transaction>>,
    mined_broadcasts: Mutex<HashSet<Txid>>
}

impl DemoChain {
    pub fn new() -> Self {
        Self {
            tip_height: Mutex::new(START_HEIGHT),
            funds: Mutex::new(HashMap::new()),
            pending_funds: Mutex::new(HashMap::new()),
            broadcast_log: Mutex::new(Vec::new()),
            mined_broadcasts: Mutex::new(HashSet::new())
        }
    }

    pub fn fund(&self, script_pubkey: ScriptBuf, amount: Amount) {
        self.funds
            .lock()
            .unwrap()
            .insert(script_pubkey, amount);
    }

    pub fn fund_pending(&self, script_pubkey: ScriptBuf, amount: Amount) {
        self.pending_funds
           .lock()
           .unwrap()
            .insert(script_pubkey, amount);
    }

    pub fn mine(&self, n: u32) {
        *self.tip_height.lock().unwrap() = n + 1;
    }

    pub fn confirm_pending(&self) {
        let pending = std::mem::take(&mut *self.pending_funds.lock().unwrap());
        self.funds.lock().unwrap().extend(pending);

        let unmined: Vec<Txid> = self
            .broadcast_log
            .lock()
            .unwrap()
            .iter()
            .map(Transaction::compute_txid)
            .collect();

        let mut mined = self.mined_broadcasts.lock().unwrap();
        for txid in unmined {
            mined.insert(txid);
        }

        drop(mined);

        self.mine(1)
    }

    pub fn broadcast_log(&self) -> Vec<Transaction> {
        self.broadcast_log
            .lock()
            .unwrap().clone()
    }

    pub fn tip(&self) -> BlockId {
        let height = *self.tip_height.lock().unwrap();

        BlockId {
            height,
            hash: block_hash(height)
        }
    }
    
    pub fn checkpoints(&self) -> CheckPoint {
        let tip = *self.tip_height.lock().unwrap();
        let ids = (0..=tip).map(|height| BlockId {
            height,
            hash: block_hash(height)
        });
        CheckPoint::from_block_ids(ids)
            .expect("a non-empty, ascending block id list is always a valid chain")
    }
}

impl ChainBackend for DemoChain {
    type Error = DemoChainError;

    fn full_scan(
        &self, 
        mut request: FullScanRequest<KeychainKind>, 
        stop_gap: usize
    ) -> Result<FullScanResponse<KeychainKind>, Self::Error> {
        let tip = self.tip();
        let timestamp = unix_time();
        let funds = self.funds.lock().unwrap();
        let pending_funds = self.pending_funds.lock().unwrap();
        
        let mut tx_update: TxUpdate<ConfirmationBlockTime> = TxUpdate::default();
        let mut last_active_indices = BTreeMap::new();
        
        for keychain in [KeychainKind::External, KeychainKind::Internal] {
            let mut last_active = None;
            let mut gap = 0usize;
            
            for (index, spk) in request.iter_spks(keychain) {
                let mut has_history = false;
                
                if let Some(&amount) = funds.get(&spk) {
                    let tx = funding_tx(&spk, amount);
                    let txid = tx.compute_txid();
                    tx_update.anchors.insert((
                        ConfirmationBlockTime {
                            block_id: tip,
                            confirmation_time: timestamp
                        },
                        txid,
                    ));
                    has_history = true;
                }
                
                if let Some(&amount) = pending_funds.get(&spk) {
                    let tx = funding_tx(&spk, amount);
                    tx_update.txs.push(Arc::new(tx.clone()));
                    tx_update.seen_ats.insert(tx.compute_txid(), timestamp);
                    has_history = true;
                }
                
                if has_history {
                    last_active = Some(index);
                    gap = 0;
                } else {
                    gap += 1;
                    if gap >= stop_gap {
                        break;
                    }
                }
            }
            
            if let Some(index) = last_active {
                last_active_indices.insert(keychain, index);
            }
        }
        
        drop(funds);
        drop(pending_funds);

        {
            let mined = self.mined_broadcasts.lock().unwrap();
            for tx in self.broadcast_log.lock().unwrap()
                .iter() {
                let txid = tx.compute_txid();
                tx_update.txs.push(Arc::new(tx.clone()));
                
                if mined.contains(&txid) {
                    tx_update.anchors.insert((
                            ConfirmationBlockTime {
                                block_id: tip,
                                confirmation_time: timestamp
                            },
                            txid
                        ));
                } else {
                    tx_update.seen_ats.insert(tx.compute_txid(), timestamp);
                }
            }
        }
        
        Ok(FullScanResponse {
            tx_update,
            last_active_indices,
            chain_update: Some(self.checkpoints()),
        })
    }

    fn broadcast(&self, tx: &Transaction) -> Result<(), Self::Error> {
        self.broadcast_log.lock()
            .unwrap()
            .push(tx.clone());
        Ok(())
    }
}

fn block_hash(height: u32) -> BlockHash {
    let byte_array = sha256d::Hash::hash(&height.to_be_bytes()).to_byte_array();
    BlockHash::from_byte_array(byte_array)
}

fn unix_time() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

fn funding_tx(spk: &ScriptBuf, amount: Amount) -> Transaction {
    let previous_output = OutPoint {
        txid: Txid::from_byte_array(sha256d::Hash::hash(spk.as_bytes()).to_byte_array()),
        vout: 0,
    };
    
    Transaction {
        version: Version::TWO,
        lock_time: LockTime::ZERO,
        input: vec![TxIn {
            previous_output,
            script_sig: ScriptBuf::new(),
            sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
            witness: Witness::new(),
        }],
        output: vec![TxOut {
            value: amount,
            script_pubkey: spk.clone()
        }]
    }
}
