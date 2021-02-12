use bincode;
use impl_serde::serialize as bytes;
use rlp::RlpStream;
use serde::{Deserialize, Serialize};
use sha3::{Digest, Keccak256};
use solana_sdk::{
    entrypoint::ProgramResult, info, instruction::Instruction, program_error::ProgramError,
    secp256k1_program
};
use std::borrow::Cow;
use std::error::Error;

pub use ethereum_types::{Address, U256};

pub fn check_tx(raw_tx: &[u8]) -> ProgramResult {
    // let eth_tx: Result<SignedTransaction, _> = rlp::decode(&raw_tx);
    // if eth_tx.is_err() {
    //     return Err(ProgramError::InvalidInstructionData);
    // }

    // let tx = eth_tx.unwrap();
    // info!(&("         to: ".to_owned() + &tx.transaction.to.unwrap().to_string()));
    // info!(&("      nonce: ".to_owned() + &tx.transaction.nonce.to_string()));
    // info!(&("        gas: ".to_owned() + &tx.transaction.gas.to_string()));
    // info!(&("  gas_price: ".to_owned() + &tx.transaction.gas_price.to_string()));
    // info!(&("      value: ".to_owned() + &tx.transaction.value.to_string()));
    // info!(&("       data: ".to_owned() + &hex::encode(&tx.transaction.data.0)));
    // info!(&("          v: ".to_owned() + &tx.v.to_string()));
    // info!(&("          r: ".to_owned() + &tx.r.to_string()));
    // info!(&("          s: ".to_owned() + &tx.s.to_string()));

    // let from = get_tx_sender(&tx).unwrap();
    // info!(&("       from: ".to_owned() + &from.to_string()));

    Ok(())
}

#[derive(Default, Serialize, Deserialize, Debug)]
struct SecpSignatureOffsets {
    signature_offset: u16, // offset to [signature,recovery_id] of 64+1 bytes
    signature_instruction_index: u8,
    eth_address_offset: u16, // offset to eth_address of 20 bytes
    eth_address_instruction_index: u8,
    message_data_offset: u16, // offset to start of message data
    message_data_size: u16,   // size of message data
    message_instruction_index: u8,
}

pub fn make_secp256k1_instruction(message: &[u8], sign: &[u8], eth_addr: &[u8]) -> Instruction {
    let mut instruction_data = vec![];
    let data_start = 1 + 11;
    // instruction_data.resize(data_start + message.len() + sign.len() + eth_addr.len(), 0);

    let message_data_offset = data_start;
    let signature_offset = message_data_offset + message.len();
    let eth_address_offset = signature_offset + sign.len();

    let offsets = SecpSignatureOffsets {
        signature_offset: signature_offset as u16,
        signature_instruction_index: 0,
        eth_address_offset: eth_address_offset as u16,
        eth_address_instruction_index: 0,
        message_data_offset: message_data_offset as u16,
        message_data_size: message.len() as u16,
        message_instruction_index: 0,
    };

    let bin_offsets = bincode::serialize(&offsets).unwrap();

    instruction_data.push(1);
    instruction_data.extend_from_slice(&bin_offsets);
    instruction_data.extend_from_slice(&message);
    instruction_data.extend_from_slice(&sign);
    instruction_data.extend_from_slice(&eth_addr);
    
    info!(&("instruction_data: ".to_owned() + &hex::encode(&instruction_data)));

    Instruction {
        program_id: secp256k1_program::id(),
        accounts: vec![],
        data: instruction_data,
    }
}

// /// Hex-serialized shim for `Vec<u8>`.
// #[derive(Serialize, Deserialize, Debug, Hash, PartialOrd, Ord, PartialEq, Eq, Clone, Default)]
// pub struct Bytes(#[serde(with = "bytes")] pub Vec<u8>);
// impl From<Vec<u8>> for Bytes {
//     fn from(s: Vec<u8>) -> Self {
//         Bytes(s)
//     }
// }

// impl std::ops::Deref for Bytes {
//     type Target = [u8];
//     fn deref(&self) -> &[u8] {
//         &self.0[..]
//     }
// }

// #[derive(Clone)]
// pub struct Transaction {
//     pub from: Address,
//     pub to: Option<Address>,
//     pub nonce: U256,
//     pub gas: U256,
//     pub gas_price: U256,
//     pub value: U256,
//     pub data: Bytes,
// }

// #[derive(Clone)]
// pub struct SignedTransaction<'a> {
//     pub transaction: Cow<'a, Transaction>,
//     pub v: u64,
//     pub r: U256,
//     pub s: U256,
// }

// mod replay_protection {
//     /// Adds chain id into v
//     pub fn add(v: u8, chain_id: u64) -> u64 {
//         v as u64 + 35 + chain_id * 2
//     }

//     /// Extracts chain_id from v
//     pub fn chain_id(v: u64) -> Option<u64> {
//         match v {
//             v if v >= 35 => Some((v - 35) / 2),
//             _ => None,
//         }
//     }
// }

// impl<'a> SignedTransaction<'a> {
//     pub fn new(
//         transaction: Cow<'a, Transaction>,
//         chain_id: u64,
//         v: u8,
//         r: [u8; 32],
//         s: [u8; 32],
//     ) -> Self {
//         let v = replay_protection::add(v, chain_id);
//         let r = U256::from_big_endian(&r);
//         let s = U256::from_big_endian(&s);

//         Self {
//             transaction,
//             v,
//             r,
//             s,
//         }
//     }

//     pub fn network_id(&self) -> Option<U256> {
//         if self.r == U256::zero() && self.s == U256::zero() {
//             Some(U256::from(self.v.clone()))
//         } else if self.v == 27u32.into() || self.v == 28u32.into() {
//             None
//         } else {
//             Some(((U256::from(self.v.clone()) - 1u32) / 2u32) - 17u32)
//         }
//     }
// }

// fn debug(s: &str, err: rlp::DecoderError) -> rlp::DecoderError {
//     // log::error!("Error decoding field: {}: {:?}", s, err);
//     err
// }

// impl<'a> rlp::Decodable for SignedTransaction<'a> {
//     fn decode(d: &rlp::Rlp) -> Result<Self, rlp::DecoderError> {
//         if d.item_count()? != 9 {
//             return Err(rlp::DecoderError::RlpIncorrectListLen);
//         }

//         Ok(SignedTransaction {
//             transaction: Cow::Owned(Transaction {
//                 nonce: d.val_at(0).map_err(|e| debug("nonce", e))?,
//                 gas_price: d.val_at(1).map_err(|e| debug("gas_price", e))?,
//                 gas: d.val_at(2).map_err(|e| debug("gas", e))?,
//                 to: {
//                     let to = d.at(3).map_err(|e| debug("to", e))?;
//                     if to.is_empty() {
//                         if to.is_data() {
//                             None
//                         } else {
//                             return Err(rlp::DecoderError::RlpExpectedToBeData);
//                         }
//                     } else {
//                         Some(to.as_val().map_err(|e| debug("to", e))?)
//                     }
//                 },
//                 from: Default::default(),
//                 value: d.val_at(4).map_err(|e| debug("value", e))?,
//                 data: d.val_at::<Vec<u8>>(5).map_err(|e| debug("data", e))?.into(),
//             }),
//             v: d.val_at(6).map_err(|e| debug("v", e))?,
//             r: d.val_at(7).map_err(|e| debug("r", e))?,
//             s: d.val_at(8).map_err(|e| debug("s", e))?,
//         })
//     }
// }

// impl rlp::Encodable for Transaction {
//     fn rlp_append(&self, s: &mut RlpStream) {
//         s.begin_list(6);
//         s.append(&self.nonce);
//         s.append(&self.gas_price);
//         s.append(&self.gas);
//         match self.to.as_ref() {
//             None => s.append(&""),
//             Some(addr) => s.append(addr),
//         };
//         s.append(&self.value);
//         s.append(&self.data.0);
//     }
// }

// impl<'a> rlp::Encodable for SignedTransaction<'a> {
//     fn rlp_append(&self, s: &mut RlpStream) {
//         s.begin_list(9);
//         s.append(&self.transaction.nonce);
//         s.append(&self.transaction.gas_price);
//         s.append(&self.transaction.gas);
//         match self.transaction.to.as_ref() {
//             None => s.append(&""),
//             Some(addr) => s.append(addr),
//         };
//         s.append(&self.transaction.value);
//         s.append(&self.transaction.data.0);
//         s.append(&self.v);
//         s.append(&self.r);
//         s.append(&self.s);
//     }
// }

// //let data = vec![0x83, b'c', b'a', b't'];
// //let decoded: SignedTransaction = rlp::decode(&data).unwrap();

// /// Pad bytes with zeros at the beggining.
// pub fn zpad(bytes: &[u8], len: usize) -> Vec<u8> {
//     if bytes.len() >= len {
//         return bytes.to_vec();
//     }
//     let mut pad = vec![0u8; len - bytes.len()];
//     pad.extend(bytes);
//     pad
// }

// #[derive(Debug)]
// pub enum GetTxError {
//     InvalidNetworkId,
//     InvalidV,
//     InvalidSignatureValues,
//     RecoveryIdFail,
//     MessageParseFail,
//     SignatureParseFail,
//     CannotRecover,
// }

// pub fn get_tx_sender(tx: &SignedTransaction) -> Result<Address, GetTxError> {
//     use secp256k1::{recover, Message, RecoveryId, Signature};

//     if tx.r == U256::zero() {
//         return Ok(Address::from([0xffu8; 20]));
//     }
//     info!("get_tx_sender");

//     let (vee, sig_hash) = if tx.v == 27u32.into() || tx.v == 28u32.into() {
//         let vee = tx.v.clone();
//         let rlp_data = rlp::encode(tx.transaction.as_ref());
//         let sig_hash = Keccak256::digest(&rlp_data);
//         (vee, sig_hash)
//     } else if tx.v >= 37u32.into() {
//         let network_id = tx.network_id();
//         if network_id.is_none() {
//             return Ok(Address::from([0xffu8; 20]));
//         }
//         let vee = (U256::from(tx.v.clone()) - (network_id.unwrap() * 2u32) - 8u32).as_u64();
//         if vee != 27u32.into() && vee != 28u32.into() {
//             return Ok(Address::from([0xffu8; 20]));
//         }

//         let rlp_data = rlp::encode(tx.transaction.as_ref());
//         let sig_hash = Keccak256::digest(&rlp_data);
//         (vee, sig_hash)
//     } else {
//         return Err(GetTxError::InvalidV);
//     };
//     info!("vee, sig_hash");

//     let SECPK1N: U256 = U256::from_dec_str(
//         "115792089237316195423570985008687907852837564279074904382605163141518161494337",
//     )
//     .unwrap();
//     if tx.r >= SECPK1N || tx.s >= SECPK1N || tx.r == U256::zero() || tx.s == U256::zero() {
//         return Err(GetTxError::InvalidSignatureValues);
//     }

//     // Prepare compact signature that consists of (r, s) padded to 32 bytes to make 64 bytes data
//     let mut r_bytes: [u8; 32] = [0; 32];
//     tx.r.to_big_endian(&mut r_bytes);
//     let r = zpad(&r_bytes, 32);
//     debug_assert_eq!(r.len(), 32);
//     let mut s_bytes: [u8; 32] = [0; 32];
//     tx.s.to_big_endian(&mut s_bytes);
//     let s = zpad(&s_bytes, 32);
//     debug_assert_eq!(s.len(), 32);

//     // Join together rs into a compact signature
//     let mut compact_bytes: Vec<u8> = Vec::new();
//     compact_bytes.extend(r);
//     compact_bytes.extend(s);
//     debug_assert_eq!(compact_bytes.len(), 64);

//     let rid_res = RecoveryId::parse_rpc(vee as u8);
//     if rid_res.is_err() {
//         return Err(GetTxError::RecoveryIdFail);
//     }
//     let rid = rid_res.unwrap();

//     let msg_res = Message::parse_slice(&sig_hash);
//     if msg_res.is_err() {
//         return Err(GetTxError::MessageParseFail);
//     }
//     let msg = msg_res.unwrap();

//     let sign_res = Signature::parse_slice(&compact_bytes);
//     if sign_res.is_err() {
//         return Err(GetTxError::SignatureParseFail);
//     }
//     let sign = sign_res.unwrap();

//     info!("b recover");

//     let rec_res = recover(&msg, &sign, &rid);
//     if rec_res.is_err() {
//         return Err(GetTxError::CannotRecover);
//     }
//     info!("a recover");

//     let pk = rec_res.unwrap();
//     let pk_data = pk.serialize();
//     let sender = Keccak256::digest(&pk_data);
//     debug_assert_eq!(sender.len(), 32);
//     return Ok(Address::from_slice(&sender));
// }
