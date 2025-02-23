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
use std::fmt;

use anyhow::{anyhow, Result};
use darkfi::{
    tx::Transaction,
    util::parse::encode_base10,
    zk::{halo2::Field, proof::ProvingKey, vm::ZkCircuit, vm_heap::empty_witnesses, Proof},
    zkas::ZkBinary,
};
use darkfi_money_contract::{
    client::{swap_v1::SwapCallBuilder, MoneyNote},
    model::{Coin, MoneyTransferParamsV1},
    MoneyFunction, MONEY_CONTRACT_ZKAS_BURN_NS_V1, MONEY_CONTRACT_ZKAS_MINT_NS_V1,
};
use darkfi_sdk::{
    crypto::{
        contract_id::MONEY_CONTRACT_ID,
        pedersen::{pedersen_commitment_base, pedersen_commitment_u64},
        poseidon_hash, PublicKey, SecretKey, TokenId,
    },
    pasta::pallas,
    tx::ContractCall,
};
use darkfi_serial::{deserialize, Encodable, SerialDecodable, SerialEncodable};
use rand::rngs::OsRng;

use super::Drk;

#[derive(Debug, Clone, SerialEncodable, SerialDecodable)]
/// Half of the swap data, includes the coin that is supposed to be sent,
/// and the coin that is supposed to be received.
pub struct PartialSwapData {
    params: MoneyTransferParamsV1,
    proofs: Vec<Proof>,
    value_pair: (u64, u64),
    token_pair: (TokenId, TokenId),
    value_blinds: Vec<pallas::Scalar>,
    token_blinds: Vec<pallas::Scalar>,
}

impl fmt::Display for PartialSwapData {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let s =
            format!(
            "{:#?}\nValue pair: {}:{}\nToken pair: {}:{}\nValue blinds: {:?}\nToken blinds: {:?}\n",
            self.params, self.value_pair.0, self.value_pair.1, self.token_pair.0, self.token_pair.1,
            self.value_blinds, self.token_blinds,
        );

        write!(f, "{}", s)
    }
}

impl Drk {
    /// Initialize the first half of an atomic swap
    pub async fn init_swap(
        &self,
        value_send: u64,
        token_send: TokenId,
        value_recv: u64,
        token_recv: TokenId,
    ) -> Result<PartialSwapData> {
        // First we'll fetch all of our unspent coins from the wallet.
        let mut owncoins = self.get_coins(false).await?;
        // Then we see if we have one that we can send.
        owncoins.retain(|x| {
            x.0.note.value == value_send &&
                x.0.note.token_id == token_send &&
                x.0.note.spend_hook == pallas::Base::zero()
        });

        if owncoins.is_empty() {
            return Err(anyhow!(
                "Did not find any unspent coins of value {} and token_id {}",
                value_send,
                token_send
            ))
        }

        // If there are any, we'll just spend the first one we see.
        let burn_coin = owncoins[0].0.clone();

        // Fetch our default address
        let address = self.wallet_address(1).await?;

        // We'll also need our Merkle tree
        let tree = self.get_money_tree().await?;

        let contract_id = *MONEY_CONTRACT_ID;

        // Now we need to do a lookup for the zkas proof bincodes, and create
        // the circuit objects and proving keys so we can build the transaction.
        // We also do this through the RPC.
        let zkas_bins = self.lookup_zkas(&contract_id).await?;

        let Some(mint_zkbin) = zkas_bins.iter().find(|x| x.0 == MONEY_CONTRACT_ZKAS_MINT_NS_V1)
        else {
            return Err(anyhow!("Mint circuit not found"))
        };

        let Some(burn_zkbin) = zkas_bins.iter().find(|x| x.0 == MONEY_CONTRACT_ZKAS_BURN_NS_V1)
        else {
            return Err(anyhow!("Burn circuit not found"))
        };

        let mint_zkbin = ZkBinary::decode(&mint_zkbin.1)?;
        let burn_zkbin = ZkBinary::decode(&burn_zkbin.1)?;

        let k = 13;
        let mint_circuit = ZkCircuit::new(empty_witnesses(&mint_zkbin), mint_zkbin.clone());
        let burn_circuit = ZkCircuit::new(empty_witnesses(&burn_zkbin), burn_zkbin.clone());

        // Since we're creating the first half, we generate the blinds.
        let value_blinds = [pallas::Scalar::random(&mut OsRng), pallas::Scalar::random(&mut OsRng)];
        let token_blinds = [pallas::Scalar::random(&mut OsRng), pallas::Scalar::random(&mut OsRng)];

        // Now we should have everything we need to build the swap half
        eprintln!("Creating Mint and Burn circuit proving keys");
        let builder = SwapCallBuilder {
            pubkey: address,
            value_send,
            token_id_send: token_send,
            value_recv,
            token_id_recv: token_recv,
            user_data_blind_send: pallas::Base::random(&mut OsRng), // <-- FIXME: Perhaps should be passed in
            spend_hook_recv: pallas::Base::zero(), // <-- FIXME: Should be passed in
            user_data_recv: pallas::Base::zero(),  // <-- FIXME: Should be passed in
            value_blinds,
            token_blinds,
            coin: burn_coin,
            tree,
            mint_zkbin,
            mint_pk: ProvingKey::build(k, &mint_circuit),
            burn_zkbin,
            burn_pk: ProvingKey::build(k, &burn_circuit),
        };

        eprintln!("Building first half of the swap transaction");
        let debris = builder.build()?;

        // Now we have the half, so we can build `PartialSwapData` and return it.
        let ret = PartialSwapData {
            params: debris.params,
            proofs: debris.proofs,
            value_pair: (value_send, value_recv),
            token_pair: (token_send, token_recv),
            value_blinds: value_blinds.to_vec(),
            token_blinds: token_blinds.to_vec(),
        };

        Ok(ret)
    }

    /// Create a full transaction by inspecting and verifying given partial swap data,
    /// making the other half, and joining all this into a `Transaction` object.
    pub async fn join_swap(&self, partial: PartialSwapData) -> Result<Transaction> {
        // Our side of the tx in the pairs is the second half, so we try to find
        // an unspent coin like that in our wallet.
        let mut owncoins = self.get_coins(false).await?;
        owncoins.retain(|x| {
            x.0.note.value == partial.value_pair.1 && x.0.note.token_id == partial.token_pair.1
        });

        if owncoins.is_empty() {
            return Err(anyhow!(
                "Did not find any unspent coins of value {} and token_id {}",
                partial.value_pair.1,
                partial.token_pair.1
            ))
        }

        // If there are any, we'll just spend the first one we see.
        let burn_coin = owncoins[0].0.clone();

        // Fetch our default address // FIXME: Should actually be getting is_default
        let address = self.wallet_address(1).await?;

        // We'll also need our Merkle tree
        let tree = self.get_money_tree().await?;

        let contract_id = *MONEY_CONTRACT_ID;

        // Now we need to do a lookup for the zkas proof bincodes, and create
        // the circuit objects and proving keys so we can build the transaction.
        // We also do this through the RPC.
        let zkas_bins = self.lookup_zkas(&contract_id).await?;

        let Some(mint_zkbin) = zkas_bins.iter().find(|x| x.0 == MONEY_CONTRACT_ZKAS_MINT_NS_V1)
        else {
            return Err(anyhow!("Mint circuit not found"))
        };

        let Some(burn_zkbin) = zkas_bins.iter().find(|x| x.0 == MONEY_CONTRACT_ZKAS_BURN_NS_V1)
        else {
            return Err(anyhow!("Burn circuit not found"))
        };

        let mint_zkbin = ZkBinary::decode(&mint_zkbin.1)?;
        let burn_zkbin = ZkBinary::decode(&burn_zkbin.1)?;

        let k = 13;
        let mint_circuit = ZkCircuit::new(empty_witnesses(&mint_zkbin), mint_zkbin.clone());
        let burn_circuit = ZkCircuit::new(empty_witnesses(&burn_zkbin), burn_zkbin.clone());

        // TODO: Maybe some kind of verification at this point

        // Now we should have everything we need to build the swap half
        eprintln!("Creating Mint and Burn circuit proving keys");
        let builder = SwapCallBuilder {
            pubkey: address,
            value_send: partial.value_pair.1,
            token_id_send: partial.token_pair.1,
            value_recv: partial.value_pair.0,
            token_id_recv: partial.token_pair.0,
            user_data_blind_send: pallas::Base::random(&mut OsRng), // <-- FIXME: Perhaps should be passed in
            spend_hook_recv: pallas::Base::zero(), // <-- FIXME: Should be passed in
            user_data_recv: pallas::Base::zero(),  // <-- FIXME: Should be passed in
            value_blinds: [partial.value_blinds[1], partial.value_blinds[0]],
            token_blinds: [partial.token_blinds[1], partial.token_blinds[0]],
            coin: burn_coin,
            tree,
            mint_zkbin,
            mint_pk: ProvingKey::build(k, &mint_circuit),
            burn_zkbin,
            burn_pk: ProvingKey::build(k, &burn_circuit),
        };

        eprintln!("Building second half of the swap transaction");
        let debris = builder.build()?;

        let full_params = MoneyTransferParamsV1 {
            clear_inputs: vec![],
            inputs: vec![partial.params.inputs[0].clone(), debris.params.inputs[0].clone()],
            outputs: vec![partial.params.outputs[0].clone(), debris.params.outputs[0].clone()],
        };

        let full_proofs = vec![
            partial.proofs[0].clone(),
            debris.proofs[0].clone(),
            partial.proofs[1].clone(),
            debris.proofs[1].clone(),
        ];

        let mut data = vec![MoneyFunction::OtcSwapV1 as u8];
        full_params.encode(&mut data)?;
        let mut tx = Transaction {
            calls: vec![ContractCall { contract_id, data }],
            proofs: vec![full_proofs],
            signatures: vec![],
        };
        eprintln!("Signing swap transaction");
        let sigs = tx.create_sigs(&mut OsRng, &[debris.signature_secret])?;
        tx.signatures = vec![sigs];

        Ok(tx)
    }

    /// Inspect and verify a given swap (half or full) transaction
    pub async fn inspect_swap(&self, bytes: Vec<u8>) -> Result<()> {
        let mut full: Option<Transaction> = None;
        let mut half: Option<PartialSwapData> = None;

        if let Ok(v) = deserialize(&bytes) {
            full = Some(v)
        };

        match deserialize(&bytes) {
            Ok(v) => half = Some(v),
            Err(_) => {
                if full.is_none() {
                    return Err(anyhow!("Failed to deserialize to Transaction or PartialSwapData"))
                }
            }
        }

        if let Some(tx) = full {
            // We're inspecting a full transaction
            if tx.calls.len() != 1 {
                eprintln!(
                    "Found {} contract calls in the transaction, there should be 1",
                    tx.calls.len()
                );
                return Err(anyhow!("Inspection failed"))
            }

            let params: MoneyTransferParamsV1 = deserialize(&tx.calls[0].data[1..])?;
            eprintln!("Parameters:\n{:#?}", params);

            if params.inputs.len() != 2 {
                eprintln!("Found {} inputs, there should be 2", params.inputs.len());
                return Err(anyhow!("Inspection failed"))
            }

            if params.outputs.len() != 2 {
                eprintln!("Found {} outputs, there should be 2", params.outputs.len());
                return Err(anyhow!("Inspection failed"))
            }

            // Try to decrypt one of the outputs.
            let secret_keys = self.get_money_secrets().await?;
            let mut skey: Option<SecretKey> = None;
            let mut note: Option<MoneyNote> = None;
            let mut output_idx = 0;

            for output in &params.outputs {
                eprintln!("Trying to decrypt note in output {}", output_idx);

                for secret in &secret_keys {
                    if let Ok(d_note) = output.note.decrypt::<MoneyNote>(secret) {
                        let s: SecretKey = deserialize(&d_note.memo)?;
                        skey = Some(s);
                        note = Some(d_note);
                        eprintln!("Successfully decrypted and found an ephemeral secret");
                        break
                    }
                }

                if note.is_some() {
                    break
                }

                output_idx += 1;
            }

            let Some(note) = note else {
                eprintln!("Error: Could not decrypt notes of either output");
                return Err(anyhow!("Inspection failed"))
            };

            eprintln!(
                "Output[{}] value: {} ({})",
                output_idx,
                note.value,
                encode_base10(note.value, 8)
            );
            eprintln!("Output[{}] token ID: {}", output_idx, note.token_id);

            let skey = skey.unwrap();
            let (pub_x, pub_y) = PublicKey::from_secret(skey).xy();
            let coin = Coin::from(poseidon_hash([
                pub_x,
                pub_y,
                pallas::Base::from(note.value),
                note.token_id.inner(),
                note.serial,
            ]));

            if coin == params.outputs[output_idx].coin {
                eprintln!("Output[{}] coin matches decrypted note metadata", output_idx);
            } else {
                eprintln!("Error: Output[{}] coin does not match note metadata", output_idx);
                return Err(anyhow!("Inspection failed"))
            }

            let valcom = pedersen_commitment_u64(note.value, note.value_blind);
            let tokcom = pedersen_commitment_base(note.token_id.inner(), note.token_blind);

            if valcom != params.outputs[output_idx].value_commit {
                eprintln!(
                    "Error: Output[{}] value commitment does not match note metadata",
                    output_idx
                );
                return Err(anyhow!("Inspection failed"))
            }

            if tokcom != params.outputs[output_idx].token_commit {
                eprintln!(
                    "Error: Output[{}] token commitment does not match note metadata",
                    output_idx
                );
                return Err(anyhow!("Inspection failed"))
            }

            eprintln!("Value and token commitments match decrypted note metadata");

            // Verify that the output commitments match the other input commitments
            match output_idx {
                0 => {
                    if valcom != params.inputs[1].value_commit ||
                        tokcom != params.inputs[1].token_commit
                    {
                        eprintln!("Error: Value/Token commits of output[0] do not match input[1]");
                        return Err(anyhow!("Inspection failed"))
                    }
                }
                1 => {
                    if valcom != params.inputs[0].value_commit ||
                        tokcom != params.inputs[0].token_commit
                    {
                        eprintln!("Error: Value/Token commits of output[1] do not match input[0]");
                        return Err(anyhow!("Inspection failed"))
                    }
                }
                _ => unreachable!(),
            }

            eprintln!("Found matching pedersen commitments for outputs and inputs");

            // TODO: Verify signature
            // TODO: Verify ZK proofs
            return Ok(())
        }

        // Inspect PartialSwapData
        let partial = half.unwrap();
        eprintln!("{}", partial);

        Ok(())
    }

    /// Sign a given transaction by retrieving the secret key from the encrypted
    /// note and prepending it to the transaction's signatures.
    pub async fn sign_swap(&self, tx: &mut Transaction) -> Result<()> {
        // We need our secret keys to try and decrypt the note
        let secret_keys = self.get_money_secrets().await?;
        let params: MoneyTransferParamsV1 = deserialize(&tx.calls[0].data[1..])?;

        // Our output should be outputs[0] so we try to decrypt that.
        let encrypted_note = &params.outputs[0].note;

        eprintln!("Trying to decrypt note in outputs[0]");
        let mut skey = None;

        for secret in &secret_keys {
            if let Ok(note) = encrypted_note.decrypt::<MoneyNote>(secret) {
                let s: SecretKey = deserialize(&note.memo)?;
                eprintln!("Successfully decrypted and found an ephemeral secret");
                skey = Some(s);
                break
            }
        }

        let Some(skey) = skey else {
            eprintln!("Error: Failed to decrypt note with any of our secret keys");
            return Err(anyhow!("Failed to decrypt note with any of our secret keys"))
        };

        eprintln!("Signing swap transaction");
        let sigs = tx.create_sigs(&mut OsRng, &[skey])?;
        tx.signatures[0].insert(0, sigs[0]);

        Ok(())
    }
}
