// Modern, minimalistic & standard-compliant Bitcoin library.
//
// SPDX-License-Identifier: Apache-2.0
//
// Designed in 2019-2025 by Dr Maxim Orlovsky <orlovsky@lnp-bp.org>
// Written in 2024-2025 by Dr Maxim Orlovsky <orlovsky@lnp-bp.org>
//
// Copyright (C) 2019-2024 LNP/BP Standards Association, Switzerland.
// Copyright (C) 2024-2025 LNP/BP Labs, Institute for Distributed and Cognitive Systems (InDCS).
// Copyright (C) 2019-2025 Dr Maxim Orlovsky.
// All rights under the above copyrights are reserved.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use std::num::ParseIntError;
use std::str::FromStr;

use bc::Txid;
use derive::{
    Address, AddressNetwork, AddressParseError, Keychain, LockTime, Network, NormalIndex, Outpoint,
    Sats, ScriptPubkey, SeqNo, Terminal, Vout,
};
use descriptors::Descriptor;

use crate::{Prevout, Psbt, PsbtError, PsbtVer, UnsignedTx};

#[derive(Clone, Debug, Display, Error, From)]
#[display(doc_comments)]
pub enum ConstructionError {
    #[display(inner)]
    Psbt(PsbtError),

    /// the input spending {0} is not known for the current wallet.
    UnknownInput(Outpoint),

    /// impossible to construct a transaction having no inputs.
    NoInputs,

    /// the total payment amount ({0} sats) exceeds the number of sats in existence.
    Overflow(Sats),

    /// attempt to spend more than present in transaction inputs. Total transaction inputs are
    /// {input_value} sats, but the output is {output_value} sats.
    OutputExceedsInputs {
        input_value: Sats,
        output_value: Sats,
    },

    /// not enough funds to pay the fee of {fee} sats; the sum of inputs is {input_value} sats,
    /// and outputs spend {output_value} sats out of them.
    NoFundsForFee {
        input_value: Sats,
        output_value: Sats,
        fee: Sats,
    },

    /// network for address {0} mismatches the one used by the wallet.
    NetworkMismatch(Address),
}

#[derive(Clone, Debug, Display, Error, From)]
#[display(doc_comments)]
pub enum BeneficiaryParseError {
    #[display("invalid format of the invoice")]
    InvalidFormat,

    #[from]
    Int(ParseIntError),

    #[from]
    Address(AddressParseError),
}

#[derive(Copy, Clone, Eq, PartialEq, Debug, Display, From)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum Payment {
    #[from]
    #[display(inner)]
    Fixed(Sats),
    #[display("MAX")]
    Max,
}

impl Payment {
    #[inline]
    pub fn sats(&self) -> Option<Sats> {
        match self {
            Payment::Fixed(sats) => Some(*sats),
            Payment::Max => None,
        }
    }

    #[inline]
    pub fn unwrap_or(&self, default: impl Into<Sats>) -> Sats {
        self.sats().unwrap_or(default.into())
    }

    #[inline]
    pub fn is_max(&self) -> bool { *self == Payment::Max }
}

impl FromStr for Payment {
    type Err = ParseIntError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s == "MAX" {
            return Ok(Payment::Max);
        }
        Sats::from_str(s).map(Payment::Fixed)
    }
}

#[derive(Copy, Clone, Eq, PartialEq, Debug, Display)]
#[display("{amount}@{address}", alt = "bitcoin:{address}?amount={amount}")]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Beneficiary {
    pub address: Address,
    pub amount: Payment,
}

impl Beneficiary {
    #[inline]
    pub fn new(address: Address, amount: impl Into<Payment>) -> Self {
        Beneficiary {
            address,
            amount: amount.into(),
        }
    }
    #[inline]
    pub fn with_max(address: Address) -> Self {
        Beneficiary {
            address,
            amount: Payment::Max,
        }
    }
    #[inline]
    pub fn is_max(&self) -> bool { self.amount.is_max() }
    #[inline]
    pub fn script_pubkey(&self) -> ScriptPubkey { self.address.script_pubkey() }
}

impl FromStr for Beneficiary {
    type Err = BeneficiaryParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (amount, beneficiary) =
            s.split_once('@').ok_or(BeneficiaryParseError::InvalidFormat)?;
        Ok(Beneficiary::new(Address::from_str(beneficiary)?, Payment::from_str(amount)?))
    }
}

#[derive(Copy, Clone, PartialEq, Debug)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct TxParams {
    pub fee: Sats,
    pub lock_time: Option<LockTime>,
    pub seq_no: SeqNo,
    pub change_shift: bool,
    pub change_keychain: Keychain,
}

impl TxParams {
    pub fn with(fee: Sats) -> Self {
        TxParams {
            fee,
            lock_time: None,
            seq_no: SeqNo::from_consensus_u32(0),
            change_shift: true,
            change_keychain: Keychain::INNER,
        }
    }
}

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct ChangeInfo {
    pub vout: Vout,
    pub terminal: Terminal,
}

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct PsbtMeta {
    pub network: AddressNetwork,
    pub fee: Sats,
    pub weight: u32,
    pub size: u32,
    pub change: Option<ChangeInfo>,
}

#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Utxo {
    pub outpoint: Outpoint,
    pub value: Sats,
    pub terminal: Terminal,
}

impl Utxo {
    #[inline]
    pub fn to_prevout(&self) -> Prevout { Prevout::new(self.outpoint, self.value) }
}

pub trait PsbtConstructor {
    /// NB: We need this separate parameter since otherwise we will bloat trait with generics which
    /// may be unknown upstream.
    type Key;
    type Descr: Descriptor<Self::Key>;

    fn descriptor(&self) -> &Self::Descr;
    fn prev_tx(&self, txid: Txid) -> Option<UnsignedTx>;
    fn utxo(&self, outpoint: Outpoint) -> Option<(Utxo, ScriptPubkey)>;
    fn network(&self) -> Network;
    fn next_derivation_index(&mut self, keychain: impl Into<Keychain>, shift: bool) -> NormalIndex;

    fn construct_psbt(
        &mut self,
        coins: impl IntoIterator<Item = Outpoint>,
        beneficiaries: impl IntoIterator<Item = Beneficiary>,
        params: TxParams,
    ) -> Result<(Psbt, PsbtMeta), ConstructionError> {
        let mut psbt = Psbt::create(PsbtVer::V2);

        // Set locktime
        psbt.fallback_locktime = params.lock_time;

        // Add xpubs
        for spec in self.descriptor().xpubs() {
            psbt.xpubs.insert(*spec.xpub(), spec.origin().clone());
        }

        // 1. Add inputs
        for coin in coins {
            let prev_tx = self.prev_tx(coin.txid).ok_or(ConstructionError::UnknownInput(coin))?;
            let (utxo, spk) = self.utxo(coin).ok_or_else(|| {
                ConstructionError::UnknownInput(coin)
            })?;
            if psbt.inputs().any(|inp| inp.previous_outpoint == utxo.outpoint) {
                continue;
            }
            psbt.append_input_expect(
                prev_tx,
                utxo.to_prevout(),
                self.descriptor(),
                utxo.terminal,
                spk,
                params.seq_no,
            );
        }
        if psbt.inputs().count() == 0 {
            return Err(ConstructionError::NoInputs);
        }

        // 2. Add outputs
        let input_value = psbt.input_sum();
        let mut max = Vec::new();
        let mut output_value = Sats::ZERO;
        for beneficiary in beneficiaries {
            if beneficiary.address.network != self.network().into() {
                return Err(ConstructionError::NetworkMismatch(beneficiary.address));
            }
            let amount = beneficiary.amount.unwrap_or(Sats::ZERO);
            output_value
                .checked_add_assign(amount)
                .ok_or(ConstructionError::Overflow(output_value))?;
            let out = psbt.append_output_expect(beneficiary.script_pubkey(), amount);
            if beneficiary.amount.is_max() {
                max.push(out.index());
            }
        }
        let mut remaining_value = input_value
            .checked_sub(output_value)
            .ok_or(ConstructionError::OutputExceedsInputs {
                input_value,
                output_value,
            })?
            .checked_sub(params.fee)
            .ok_or(ConstructionError::NoFundsForFee {
                input_value,
                output_value,
                fee: params.fee,
            })?;
        if !max.is_empty() {
            let portion = remaining_value / max.len();
            for out in psbt.outputs_mut() {
                if max.contains(&out.index()) {
                    out.amount = portion;
                }
            }
            remaining_value = Sats::ZERO;
        }

        // 3. Add change - only if exceeded the dust limit
        let change = if remaining_value > self.descriptor().class().dust_limit() {
            let change_index =
                self.next_derivation_index(params.change_keychain, params.change_shift);
            let change_terminal = Terminal::new(params.change_keychain, change_index);
            let change_vout = psbt
                .append_change_expect(self.descriptor(), change_terminal, remaining_value)
                .index();
            Some(ChangeInfo {
                vout: Vout::from_u32(change_vout as u32),
                terminal: change_terminal,
            })
        } else {
            None
        };

        let meta = PsbtMeta {
            network: self.network().into(),
            fee: params.fee,
            weight: 0, // TODO: Implement weight/size computation
            size: 0,
            change,
        };
        self.after_construct_psbt(&psbt, &meta);

        Ok((psbt, meta))
    }

    /// A hook which is called by the default `Self::construct_psbt` before returning the newly
    /// constructed PSBT to the caller.
    fn after_construct_psbt(&mut self, _psbt: &Psbt, _meta: &PsbtMeta) {
        // By default, we do not use the hook
    }
}
