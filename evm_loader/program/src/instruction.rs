use serde::{Serialize, Deserialize};
use solana_sdk::{
    program_error::ProgramError, pubkey::Pubkey,
    instruction::Instruction,
};
use std::convert::TryInto;
use primitive_types::{H160, H256};
use evm::backend::Log;

/// Create a new account
#[derive(Debug, PartialEq, Eq, Clone)]
pub enum EvmInstruction<'a> {
    /// Write program data into an account
    ///
    /// # Account references
    ///   0. [WRITE] Account to write to
    ///   1. [SIGNER] Signer for Ether account
    Write {
        /// Offset at which to write the given bytes
        offset: u32,
        bytes: &'a [u8],
    },

    /// Finalize an account loaded with program data for execution
    ///
    /// The exact preparation steps is loader specific but on success the loader must set the executable
    /// bit of the account.
    ///
    /// # Account references
    ///   0. [WRITE] The account to prepare for execution
    ///   1. [WRITE] Caller (Ether account)
    ///   2. [SIGNER] Signer for Ether account
    ///   3. [] Clock sysvar
    ///   4. [] Rent sysvar
    ///   ... other Ether accounts
    Finalize,

    ///
    /// Create Ethereum account (create program_address account and write data)
    /// # Account references
    ///   0. [WRITE, SIGNER] Funding account
    ///   1. [WRITE] New account (program_address(ether, nonce))
    CreateAccount {
        /// Number of lamports to transfer to the new account
        lamports: u64,

        /// Number of bytes of memory to allocate
        space: u64,

        /// Ethereum address of account
        ether: H160,

        /// Nonce for create valid program_address from ethereum address
        nonce: u8,
    },

    ///
    /// Create ethereum account with seed
    /// # Account references
    ///   0. [WRITE, SIGNER] Funding account
    ///   1. [WRITE] New account (create_with_seed(base, seed, owner)
    ///   2. [] Base (program_addres(ether, nonce))
    CreateAccountWithSeed {
        /// Base public key
        base: Pubkey,

        /// String of ASCII chars, no longer than `Pubkey::MAX_SEED_LEN`
        seed: Vec<u8>,

        /// Number of lamports to transfer to the new account
        lamports: u64,

        /// Number of bytes of memory to allocate
        space: u64,

        /// Owner program account address
        owner: Pubkey,
    },

    /// Call Ethereum-contract action
    /// # Account references
    ///   0. [WRITE] Contract for execution (Ether account)
    ///   1. [WRITE] Caller (Ether account)
    ///   2. [SIGNER] Signer for caller
    ///   3. [] Clock sysvar
    ///   ... other Ether accounts
    Call {
        /// Call data
        bytes: &'a [u8],
    },

    /// Called action return
    OnReturn {
        /// Returned data
        bytes: &'a [u8],
    },

    /// Called action event
    OnEvent {
        address: H160,
        topics: Vec<H256>,
        /// Data
        data: &'a [u8],
    },
}


impl<'a> EvmInstruction<'a> {
    pub fn unpack(input: &'a[u8]) -> Result<Self, ProgramError> {
        use ProgramError::InvalidInstructionData;

        let (&tag, rest) = input.split_first().ok_or(InvalidInstructionData)?;
        Ok(match tag {
            0 => {
                let (_, rest) = rest.split_at(3);
                let (offset, rest) = rest.split_at(4);
                let (length, rest) = rest.split_at(8);
                let offset = offset.try_into().ok().map(u32::from_le_bytes).ok_or(InvalidInstructionData)?;
                let length = length.try_into().ok().map(u64::from_le_bytes).ok_or(InvalidInstructionData)?;
                let (bytes, _) = rest.split_at(length as usize);
                EvmInstruction::Write {offset, bytes}
            },
            1 => {
                let (_, _rest) = rest.split_at(3);
                EvmInstruction::Finalize
            },
            2 => {
                let (_, rest) = rest.split_at(3);
                let (lamports, rest) = rest.split_at(8);
                let (space, rest) = rest.split_at(8);

                let lamports = lamports.try_into().ok().map(u64::from_le_bytes).ok_or(InvalidInstructionData)?;
                let space = space.try_into().ok().map(u64::from_le_bytes).ok_or(InvalidInstructionData)?;

                let (ether, rest) = rest.split_at(20);
                let ether = H160::from_slice(&*ether); //ether.try_into().map_err(|_| InvalidInstructionData)?;
                let (nonce, _rest) = rest.split_first().ok_or(InvalidInstructionData)?;
                EvmInstruction::CreateAccount {lamports, space, ether, nonce: *nonce}
            },
            3 => {
                EvmInstruction::Call {bytes: rest}
            },
            4 => {
                let (_, rest) = rest.split_at(3);
                let (base, rest) = rest.split_at(32);
                let (seed_len, rest) = rest.split_at(4);
                let (_, rest) = rest.split_at(4);  // padding
                let seed_len = seed_len.try_into().ok().map(u32::from_le_bytes).ok_or(InvalidInstructionData)?;
                let (seed, rest) = rest.split_at(seed_len as usize);

                let base = Pubkey::new(base);
                let (lamports, rest) = rest.split_at(8);
                let (space, rest) = rest.split_at(8);

                let (owner, rest) = rest.split_at(32);
                let owner = Pubkey::new(owner);

                let seed = seed.into();
                let lamports = lamports.try_into().ok().map(u64::from_le_bytes).ok_or(InvalidInstructionData)?;
                let space = space.try_into().ok().map(u64::from_le_bytes).ok_or(InvalidInstructionData)?;

                EvmInstruction::CreateAccountWithSeed {base, seed, lamports, space, owner}
            },
            5 => {
                EvmInstruction::OnReturn {bytes: rest}
            },
            6 => {
                let (address, rest) = rest.split_at(20);
                let address = H160::from_slice(&*address); //address.try_into().map_err(|_| InvalidInstructionData)?;
                
                let (topics_cnt, mut rest) = rest.split_at(8);
                let topics_cnt = topics_cnt.try_into().ok().map(u64::from_le_bytes).ok_or(InvalidInstructionData)?;
                let mut topics = Vec::new();
                for i in 1..=topics_cnt {
                    let (topic, rest2) = rest.split_at(32);
                    let topic = H256::from_slice(&*topic);
                    topics.push(topic);
                    rest = rest2;
                }
                EvmInstruction::OnEvent {address, topics, data: rest}
            },
            _ => return Err(InvalidInstructionData),
        })
    }
}

/// Creates a `OnReturn` instruction.
pub fn on_return(
    myself_program_id: &Pubkey,
    mut result: Vec<u8>
) -> Result<Instruction, ProgramError> {
    result.insert(0, 5u8);

    Ok(Instruction {
        program_id: *myself_program_id,
        accounts: [].to_vec(),
        data: result,
    })
}

/// Creates a `OnEvent` instruction.
pub fn on_event(
    myself_program_id: &Pubkey,
    log: Log
) -> Result<Instruction, ProgramError> {
    let mut data = Vec::new();
    data.insert(0, 6u8);

    data.extend_from_slice(log.address.as_bytes());

    data.extend_from_slice(&log.topics.len().to_le_bytes());
    for topic in log.topics {
        data.extend_from_slice(topic.as_bytes());
    }

    data.extend(&log.data);

    Ok(Instruction {
        program_id: *myself_program_id,
        accounts: [].to_vec(),
        data,
    })
}
