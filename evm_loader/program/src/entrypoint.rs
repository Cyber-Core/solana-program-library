//! Program entrypoint

#![cfg(feature = "program")]
#![cfg(not(feature = "no-entrypoint"))]

//use crate::{error::TokenError, processor::Processor};
//use arrayref::{array_ref, array_refs, array_mut_ref, mut_array_refs};
use std::convert::TryInto;
use solana_sdk::{
    account_info::{next_account_info, AccountInfo},
    instruction::{AccountMeta, Instruction},
    entrypoint, entrypoint::{ProgramResult},
    program_error::{ProgramError}, pubkey::Pubkey,
    program_utils::{limited_deserialize},
    loader_instruction::LoaderInstruction,
    system_instruction::{create_account, create_account_with_seed},
    program::invoke_signed, program::invoke,
    info,
};

//use crate::hamt::Hamt;
use crate::solana_backend::{
    SolanaBackend, solidity_address,
};

use crate::{
//    bump_allocator::BumpAllocator,
    instruction::EvmInstruction,
    account_data::AccountData,
    solidity_account::SolidityAccount,
};

use evm::{
//    backend::{MemoryVicinity, MemoryAccount, MemoryBackend, Apply},
    executor::{StackExecutor},
    ExitReason,
};
use primitive_types::{U256};

use std::{alloc::Layout, mem::size_of, ptr::null_mut, usize};
use solana_sdk::entrypoint::HEAP_START_ADDRESS;


use sha3::{Keccak256, Digest};
use primitive_types::H256;
fn keccak256_digest(data: &[u8]) -> H256 {
    H256::from_slice(Keccak256::digest(&data).as_slice())
}


const HEAP_LENGTH: usize = 1024*1024;

/// Developers can implement their own heap by defining their own
/// `#[global_allocator]`.  The following implements a dummy for test purposes
/// but can be flushed out with whatever the developer sees fit.
pub struct BumpAllocator;

impl BumpAllocator {
    /// Get occupied memory
    #[inline]
    pub fn occupied() -> usize {
        const POS_PTR: *mut usize = HEAP_START_ADDRESS as *mut usize;
        const TOP_ADDRESS: usize = HEAP_START_ADDRESS + HEAP_LENGTH;

        let pos = unsafe{*POS_PTR};
        if pos == 0 {0} else {TOP_ADDRESS-pos}
    }
}

unsafe impl std::alloc::GlobalAlloc for BumpAllocator {
    #[inline]
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        const POS_PTR: *mut usize = HEAP_START_ADDRESS as *mut usize;
        const TOP_ADDRESS: usize = HEAP_START_ADDRESS + HEAP_LENGTH;
        const BOTTOM_ADDRESS: usize = HEAP_START_ADDRESS + size_of::<*mut u8>();

        let mut pos = *POS_PTR;
        if pos == 0 {
            // First time, set starting position
            pos = TOP_ADDRESS;
        }
        pos = pos.saturating_sub(layout.size());
        pos &= !(layout.align().saturating_sub(1));
        if pos < BOTTOM_ADDRESS {
            return null_mut();
        }

        *POS_PTR = pos;
        pos as *mut u8
    }
    #[inline]
    unsafe fn dealloc(&self, _: *mut u8, _layout: Layout) {
        // I'm a bump allocator, I don't free
    }
}


#[cfg(target_arch = "bpf")]
#[global_allocator]
static mut A: BumpAllocator = BumpAllocator;

// Is't need to save for account:
// 1. ether: [u8;20]
// 2. nonce: u8
// 3. trx_count: u128
// 4. signer: pubkey
// 5. code_size: u32
// 6. storage (all remaining space, if code_size not equal zero)

entrypoint!(process_instruction);
fn process_instruction<'a>(
    program_id: &Pubkey,
    accounts: &'a [AccountInfo<'a>],
    instruction_data: &[u8],
) -> ProgramResult {

    let account_info_iter = &mut accounts.iter();

    let instruction = EvmInstruction::unpack(instruction_data)?;
    info!("Instruction parsed");

    let result = match instruction {
        EvmInstruction::CreateAccount {lamports, space, ether, nonce} => {
            let funding_info = next_account_info(account_info_iter)?;
            let program_info = next_account_info(account_info_iter)?;

            info!(&("Ether:".to_owned()+&(hex::encode(ether))+" "+&hex::encode([nonce])));

            let expected_address = Pubkey::create_program_address(&[ether.as_bytes(), &[nonce]], program_id)?;
            if expected_address != *program_info.key {
                return Err(ProgramError::InvalidArgument);
            };

            let program_seeds = [ether.as_bytes(), &[nonce]];
            invoke_signed(
                &create_account(funding_info.key, program_info.key, lamports, AccountData::SIZE as u64 + space, program_id),
                &accounts, &[&program_seeds[..]]
            )?;
            info!("create_account done");
            
            let mut data = program_info.data.borrow_mut();
            let account_data = AccountData {ether, nonce, trx_count: 0u64, signer: *funding_info.key, code_size: 0u32};
            account_data.pack(&mut data)?;
            Ok(())
        },
        EvmInstruction::CreateAccountWithSeed {base, seed, lamports, space, owner} => {
            let funding_info = next_account_info(account_info_iter)?;
            let created_info = next_account_info(account_info_iter)?;
            let base_info = next_account_info(account_info_iter)?;

            //info!(&("Ether:".to_owned()+&(hex::encode(ether))+" "+&hex::encode([nonce])));
            if base_info.owner != program_id {return Err(ProgramError::InvalidArgument);}
            let caller = SolidityAccount::new(base_info)?;

            let program_seeds = [caller.account_data.ether.as_bytes(), &[caller.account_data.nonce]];
            let seed = std::str::from_utf8(&seed).map_err(|_| ProgramError::InvalidArgument)?;
            info!(&lamports.to_string());
            info!(&space.to_string());
            invoke_signed(
                &create_account_with_seed(funding_info.key, created_info.key, &base, &seed, lamports, space, &owner),
                &accounts, &[&program_seeds[..]]
            )?;
            info!("create_account_with_seed done");

            Ok(())
        },
        EvmInstruction::Write {offset, bytes} => {
            let program_info = next_account_info(account_info_iter)?;
            if program_info.owner != program_id {
                return Err(ProgramError::InvalidArgument);
            }
            do_write(program_info, offset, &bytes)
        },
        EvmInstruction::Finalize => {
            do_finalize(program_id, accounts)
        },
        EvmInstruction::Call {bytes} => {
            do_call(program_id, accounts, bytes)
        },
        EvmInstruction::OnReturn {bytes} => {
            Ok(())
        },
    };

/*    let result = if program_lamports == 0 {
        do_create_account(program_id, accounts, instruction_data)
    } else {
        let account_type = {program_info.data.borrow()[0]};
        if account_type == 0 {
            let instruction: LoaderInstruction = limited_deserialize(instruction_data)
                .map_err(|_| ProgramError::InvalidInstructionData)?;

            match instruction {
                LoaderInstruction::Write {offset, bytes} => {
                    do_write(program_info, offset, &bytes)
                },
                LoaderInstruction::Finalize => {
                    info!("FinalizeInstruction");
                    do_finalize(program_id, accounts, program_info)
                },
            }
        } else {
            info!("Execute");
            do_execute(program_id, accounts, instruction_data)
        }
    };*/

    info!(&("Total memory occupied: ".to_owned() + &BumpAllocator::occupied().to_string()));
    result
}

fn do_create_account<'a>(program_id: &Pubkey, accounts: &'a [AccountInfo<'a>], instruction_data: &[u8]) -> ProgramResult {
    info!("initialize account");
/*
    // If account not initialized - we can only create it...
    let instruction: EvmInstruction = limited_deserialize(instruction_data)
            .map_err(|_| ProgramError::InvalidInstructionData)?;
    match instruction {
        EvmInstruction::CreateAccount {lamports, space, ether, nonce } => {
            let account_info_iter = &mut accounts.iter();
            let program_info = next_account_info(account_info_iter)?;
            let funding_info = next_account_info(account_info_iter)?;
            let expected_address = Pubkey::create_program_address(&[&ether, &[nonce]], program_id)?;
            if expected_address != *program_info.key {
                return Err(ProgramError::InvalidArgument);
            };
            let empty_seeds = [];
            let program_seeds = [&ether[..], &[nonce]];
            invoke_signed(
                &create_account(funding_info.key, program_info.key, lamports, space, program_id),
                &accounts, &[&empty_seeds[..], &program_seeds[..]]
            )?;
            Ok(())
        },
        _ => {Err(ProgramError::InvalidInstructionData)}
    }*/
    Err(ProgramError::InvalidInstructionData)
}

fn do_write(program_info: &AccountInfo, offset: u32, bytes: &[u8]) -> ProgramResult {
    let mut data = program_info.data.borrow_mut();
    let offset = offset as usize;
    if data.len() < offset+AccountData::SIZE + bytes.len() {
        info!("Account data too small");
        return Err(ProgramError::AccountDataTooSmall);
    }
    data[offset+AccountData::SIZE..offset+AccountData::SIZE + bytes.len()].copy_from_slice(&bytes);
    Ok(())
}

fn do_finalize<'a>(program_id: &Pubkey, accounts: &'a [AccountInfo<'a>]) -> ProgramResult {
    info!("do_finalize");
    let account_info_iter = &mut accounts.iter();
    let program_info = next_account_info(account_info_iter)?;
    let caller_info = next_account_info(account_info_iter)?;
    let signer_info = next_account_info(account_info_iter)?;
    let clock_info = next_account_info(account_info_iter)?;
    let rent_info = next_account_info(account_info_iter)?;

    if program_info.owner != program_id {
        return Err(ProgramError::InvalidArgument);
    }

    let mut backend = SolanaBackend::new(program_id, accounts, clock_info)?;
    info!("  backend initialized");

    let config = evm::Config::istanbul();
    let mut executor = StackExecutor::new(&backend, usize::max_value(), &config);
    info!("  executor initialized");

    let caller = backend.get_account_by_index(1).ok_or(ProgramError::InvalidArgument)?;

    info!("Execute transact_create");

    let code_data = {
        let data = program_info.data.borrow();
        let (_unused, rest) = data.split_at(AccountData::SIZE);
        let (code_len, rest) = rest.split_at(8);
        let code_len = code_len.try_into().ok().map(u64::from_le_bytes).unwrap();
        let (code, _rest) = rest.split_at(code_len as usize);
        code.to_vec()
    };

    let program_account = SolidityAccount::new(program_info)?;

    let exit_reason = executor.transact_create2(
            caller.get_ether(),
            U256::zero(),
            code_data,
            H256::default(), usize::max_value()
        );
    info!("  create2 done");

    if exit_reason.is_succeed() {
        info!("Succeed execution");
        let (applies, logs) = executor.deconstruct();
        backend.apply(applies, logs, false)?;
        Ok(())
    } else {
        info!("Not succeed execution");
        Err(ProgramError::InvalidInstructionData)
    }
}

fn do_call<'a>(
        program_id: &Pubkey,
        accounts: &'a [AccountInfo<'a>],
        instruction_data: &[u8],
    ) -> ProgramResult
{
    info!("do_call");
    let account_info_iter = &mut accounts.iter();
    let myself_info = next_account_info(account_info_iter)?;
    let program_info = next_account_info(account_info_iter)?;
    let caller_info = next_account_info(account_info_iter)?;
    let signer_info = next_account_info(account_info_iter)?;
    let clock_info = next_account_info(account_info_iter)?;

    let mut backend = SolanaBackend::new(program_id, accounts, accounts.last().unwrap())?;
    let config = evm::Config::istanbul();
    let mut executor = StackExecutor::new(&backend, usize::max_value(), &config);
    info!("Executor initialized");
    let contract = backend.get_account_by_index(0).ok_or(ProgramError::InvalidArgument)?;
    let caller = backend.get_account_by_index(1).ok_or(ProgramError::InvalidArgument)?;
    info!(&("   caller: ".to_owned() + &caller.get_ether().to_string()));
    info!(&(" contract: ".to_owned() + &contract.get_ether().to_string()));

    let (exit_reason, mut result) = executor.transact_call(
            caller.get_ether(),
            contract.get_ether(),
            U256::zero(),
            instruction_data.to_vec(),
            usize::max_value()
        );

    info!("Call done");
    info!(match exit_reason {
        ExitReason::Succeed(_) => {
            let (applies, logs) = executor.deconstruct();
            backend.apply(applies, logs, false)?;
            info!("Applies done");
            "succeed"
        },
        ExitReason::Error(_) => "error",
        ExitReason::Revert(_) => "revert",
        ExitReason::Fatal(_) => "fatal",
    });
    info!(&hex::encode(&result));
    
    if !exit_reason.is_succeed() {
        info!("Not succeed execution");
        return Err(ProgramError::InvalidInstructionData);
    }

    // TODO: this should be separate method in instruction.rs
    result.insert(0, 5u8);
    invoke(
        &Instruction {
            program_id: *program_id,
            accounts: [].to_vec(),
            data: result,
        },
        &accounts
    )?;

    Ok(())
}



// Pull in syscall stubs when building for non-BPF targets
//#[cfg(not(target_arch = "bpf"))]
//solana_sdk::program_stubs!();

#[cfg(test)]
mod tests {
    use super::*;
    use solana_sdk::{program_error::ProgramError, pubkey::Pubkey};

    #[test]
    fn test_write() {
        let program_id = Pubkey::new(&[0; 32]);

        let string = b"letters and such";
        assert_eq!(Ok(()), process_instruction(&program_id, &[], string));

        let emoji = "🐆".as_bytes();
        let bytes = [0xF0, 0x9F, 0x90, 0x86];
        assert_eq!(emoji, bytes);
        assert_eq!(Ok(()), process_instruction(&program_id, &[], &emoji));

        let mut bad_utf8 = bytes;
        bad_utf8[3] = 0xFF; // Invalid UTF-8 byte
        assert_eq!(
            Err(ProgramError::InvalidInstructionData),
            process_instruction(&program_id, &[], &bad_utf8)
        );
    }
}

