/* This file is part of DarkFi (https://dark.fi)
 *
 * Copyright (C) 2020-2023 Dyne.org foundation
 *
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU Affero General Public License as
 * published by the Free Software Foundation, either version 3 of the
 * License, or (at your option) any later version.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU Affero General Public License for more details.
 *
 * You should have received a copy of the GNU Affero General Public License
 * along with this program.  If not, see <https://www.gnu.org/licenses/>.
 */

use std::{collections::HashMap, io::Cursor};

use darkfi_sdk::{
    crypto::{PublicKey, CONSENSUS_CONTRACT_ID},
    pasta::pallas,
};
use darkfi_serial::{Decodable, Encodable, WriteExt};
use log::{debug, error, warn};

use crate::{
    blockchain::{BlockInfo, BlockchainOverlayPtr},
    error::TxVerifyFailed,
    runtime::vm_runtime::Runtime,
    tx::Transaction,
    util::time::TimeKeeper,
    zk::VerifyingKey,
    Error, Result,
};

/// Validate given [`BlockInfo`], and apply it to the provided overlay
pub async fn verify_block(
    overlay: &BlockchainOverlayPtr,
    time_keeper: &TimeKeeper,
    block: &BlockInfo,
    previous: Option<&BlockInfo>,
    testing_mode: bool,
) -> Result<()> {
    let block_hash = block.blockhash();
    debug!(target: "validator", "Validating block {}", block_hash);

    // Check if block already exists
    if overlay.lock().unwrap().has_block(block)? {
        return Err(Error::BlockAlreadyExists(block.blockhash().to_string()))
    }

    // Block slot must be the same as the time keeper verifying slot
    if block.header.slot != time_keeper.verifying_slot {
        return Err(Error::VerifyingSlotMissmatch())
    }

    // Validate block using its previous, excluding genesis
    if block.header.slot != 0 {
        if previous.is_none() {
            return Err(Error::BlockPreviousMissing())
        }
        block.validate(previous.unwrap())?;
    }

    // Validate proposal transaction if not in testing mode
    if !testing_mode {
        verify_proposal_transaction(overlay, time_keeper, &block.producer.proposal).await?;
    }

    // Verify transactions
    verify_transactions(overlay, time_keeper, &block.txs).await?;

    // Insert block
    overlay.lock().unwrap().add_block(block)?;

    debug!(target: "validator", "Block {} verified successfully", block_hash);
    Ok(())
}

/// Validate WASM execution, signatures, and ZK proofs for a given proposal [`Transaction`],
/// and apply it to the provided overlay.
pub async fn verify_proposal_transaction(
    overlay: &BlockchainOverlayPtr,
    time_keeper: &TimeKeeper,
    tx: &Transaction,
) -> Result<()> {
    let tx_hash = tx.hash();
    debug!(target: "validator", "Validating proposal transaction {}", tx_hash);

    // Genesis transaction must be the Transaction::default() one (empty)
    if time_keeper.verifying_slot == 0 {
        if *tx != Transaction::default() {
            error!(target: "validator", "Genesis proposal transaction is not default one");
            return Err(TxVerifyFailed::ErroneousTxs(vec![tx.clone()]).into())
        }

        return Ok(())
    }

    // Transaction must contain a single Consensus::Proposal (0x02) call
    if tx.calls.len() != 1 ||
        (tx.calls[0].contract_id != *CONSENSUS_CONTRACT_ID && tx.calls[0].data[0] != 0x02)
    {
        error!(target: "validator", "Proposal transaction is malformed");
        return Err(TxVerifyFailed::ErroneousTxs(vec![tx.clone()]).into())
    }

    // Map of ZK proof verifying keys for the current transaction batch
    let mut vks: HashMap<[u8; 32], HashMap<String, VerifyingKey>> = HashMap::new();

    // Initialize the map
    vks.insert(tx.calls[0].contract_id.to_bytes(), HashMap::new());

    // TODO: when fee is implemented, differentiate here since this transaction
    // won't have fee
    verify_transaction(overlay, time_keeper, tx, &mut vks).await?;

    debug!(target: "validator", "Proposal transaction {} verified successfully", tx_hash);

    Ok(())
}

/// Validate WASM execution, signatures, and ZK proofs for a given [`Transaction`],
/// and apply it to the provided overlay.
pub async fn verify_transaction(
    overlay: &BlockchainOverlayPtr,
    time_keeper: &TimeKeeper,
    tx: &Transaction,
    verifying_keys: &mut HashMap<[u8; 32], HashMap<String, VerifyingKey>>,
) -> Result<()> {
    let tx_hash = tx.hash();
    debug!(target: "validator", "Validating transaction {}", tx_hash);

    // Table of public inputs used for ZK proof verification
    let mut zkp_table = vec![];
    // Table of public keys used for signature verification
    let mut sig_table = vec![];

    // Iterate over all calls to get the metadata
    for (idx, call) in tx.calls.iter().enumerate() {
        debug!(target: "validator", "Executing contract call {}", idx);

        // Write the actual payload data
        let mut payload = vec![];
        payload.write_u32(idx as u32)?; // Call index
        tx.calls.encode(&mut payload)?; // Actual call data

        debug!(target: "validator", "Instantiating WASM runtime");
        let wasm = overlay.lock().unwrap().wasm_bincode.get(call.contract_id)?;

        let mut runtime =
            Runtime::new(&wasm, overlay.clone(), call.contract_id, time_keeper.clone())?;

        debug!(target: "validator", "Executing \"metadata\" call");
        let metadata = runtime.metadata(&payload)?;

        // Decode the metadata retrieved from the execution
        let mut decoder = Cursor::new(&metadata);

        // The tuple is (zkasa_ns, public_inputs)
        let zkp_pub: Vec<(String, Vec<pallas::Base>)> = Decodable::decode(&mut decoder)?;
        let sig_pub: Vec<PublicKey> = Decodable::decode(&mut decoder)?;
        // TODO: Make sure we've read all the bytes above.
        debug!(target: "validator", "Successfully executed \"metadata\" call");

        // Here we'll look up verifying keys and insert them into the per-contract map.
        debug!(target: "validator", "Performing VerifyingKey lookups from the sled db");
        for (zkas_ns, _) in &zkp_pub {
            let inner_vk_map = verifying_keys.get_mut(&call.contract_id.to_bytes()).unwrap();

            // TODO: This will be a problem in case of ::deploy, unless we force a different
            // namespace and disable updating existing circuit. Might be a smart idea to do
            // so in order to have to care less about being able to verify historical txs.
            if inner_vk_map.contains_key(zkas_ns.as_str()) {
                continue
            }

            let (_, vk) = overlay.lock().unwrap().contracts.get_zkas(&call.contract_id, zkas_ns)?;

            inner_vk_map.insert(zkas_ns.to_string(), vk);
        }

        zkp_table.push(zkp_pub);
        sig_table.push(sig_pub);

        // After getting the metadata, we run the "exec" function with the same runtime
        // and the same payload.
        debug!(target: "validator", "Executing \"exec\" call");
        let state_update = runtime.exec(&payload)?;
        debug!(target: "validator", "Successfully executed \"exec\" call");

        // If that was successful, we apply the state update in the ephemeral overlay.
        debug!(target: "validator", "Executing \"apply\" call");
        runtime.apply(&state_update)?;
        debug!(target: "validator", "Successfully executed \"apply\" call");

        // At this point we're done with the call and move on to the next one.
    }

    // When we're done looping and executing over the tx's contract calls, we now
    // move on with verification. First we verify the signatures as that's cheaper,
    // and then finally we verify the ZK proofs.
    debug!(target: "validator", "Verifying signatures for transaction {}", tx_hash);
    if sig_table.len() != tx.signatures.len() {
        error!(target: "validator", "Incorrect number of signatures in tx {}", tx_hash);
        return Err(TxVerifyFailed::MissingSignatures.into())
    }

    // TODO: Go through the ZK circuits that have to be verified and account for the opcodes.

    if let Err(e) = tx.verify_sigs(sig_table) {
        error!(target: "validator", "Signature verification for tx {} failed: {}", tx_hash, e);
        return Err(TxVerifyFailed::InvalidSignature.into())
    }

    debug!(target: "validator", "Signature verification successful");

    debug!(target: "validator", "Verifying ZK proofs for transaction {}", tx_hash);
    if let Err(e) = tx.verify_zkps(verifying_keys, zkp_table).await {
        error!(target: "validator", "ZK proof verification for tx {} failed: {}", tx_hash, e);
        return Err(TxVerifyFailed::InvalidZkProof.into())
    }

    debug!(target: "validator", "ZK proof verification successful");
    debug!(target: "validator", "Transaction {} verified successfully", tx_hash);

    Ok(())
}

/// Validate a set of [`Transaction`] in sequence and apply them if all are valid.
/// In case any of the transactions fail, they will be returned to the caller.
/// The function takes a boolean called `write` which tells it to actually write
/// the state transitions to the database.
pub async fn verify_transactions(
    overlay: &BlockchainOverlayPtr,
    time_keeper: &TimeKeeper,
    txs: &[Transaction],
) -> Result<Vec<Transaction>> {
    debug!(target: "validator", "Verifying {} transactions", txs.len());

    // Tracker for failed txs
    let mut erroneous_txs = vec![];

    // Map of ZK proof verifying keys for the current transaction batch
    let mut vks: HashMap<[u8; 32], HashMap<String, VerifyingKey>> = HashMap::new();

    // Initialize the map
    for tx in txs {
        for call in &tx.calls {
            vks.insert(call.contract_id.to_bytes(), HashMap::new());
        }
    }

    // Iterate over transactions and attempt to verify them
    for tx in txs {
        overlay.lock().unwrap().checkpoint();
        if let Err(e) = verify_transaction(overlay, time_keeper, tx, &mut vks).await {
            warn!(target: "validator", "Transaction verification failed: {}", e);
            erroneous_txs.push(tx.clone());
            // TODO: verify this works as expected
            overlay.lock().unwrap().revert_to_checkpoint()?;
        }
    }

    Ok(erroneous_txs)
}
