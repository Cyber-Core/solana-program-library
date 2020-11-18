use evm::{
    backend::{Basic, Backend, ApplyBackend, Apply, Log},
    CreateScheme,
};
use primitive_types::{H160, H256, U256};
use sha3::{Digest, Keccak256};
use solana_sdk::{
    account_info::AccountInfo,
    pubkey::Pubkey,
    program_error::ProgramError,
};
use crate::hamt::Hamt;
use crate::account_data::AccountData;

fn keccak256_digest(data: &[u8]) -> H256 {
    H256::from_slice(Keccak256::digest(&data).as_slice())
}

fn solidity_address<'a>(key: &Pubkey) -> H160 {
    H256::from_slice(key.as_ref()).into()
}

struct SolidityAccount<'a> {
    key: H160,
    account: AccountData,
    info: &'a AccountInfo<'a>,
}

pub struct SolanaBackend<'a> {
    accounts: &'a [AccountInfo<'a>],
    acc_index: Vec<(H160,&'a AccountInfo<'a>)>,
}

impl<'a> SolanaBackend<'a> {
    pub fn new(accounts: &'a [AccountInfo<'a>]) -> Self {
        let mut acc_index = Vec::with_capacity(accounts.len());
        for acc in accounts {
            acc_index.push((solidity_address(&acc.key), acc));
        };
        acc_index.sort_by_key(|v| v.0);
        Self {accounts: accounts, acc_index: acc_index}
    }

    fn add_account(&mut self, address: H160, account: &'a AccountInfo<'a>) {
        self.acc_index.push((address, account));
        self.acc_index.sort_by_key(|v| v.0);
    }

    fn get_account(&self, address: H160) -> Option<&'a AccountInfo<'a>> {
        match self.acc_index.binary_search_by_key(&address, |v| v.0) {
            Ok(pos) => Some(self.acc_index[pos].1),
            Err(_) => None,
        }
    }

    fn apply<A, I, L>(&mut self, values: A, logs: L, delete_empty: bool) -> Result<(), ProgramError>
            where
                A: IntoIterator<Item=Apply<I>>,
                I: IntoIterator<Item=(H256, H256)>,
                L: IntoIterator<Item=Log>,
    {
        for i in &self.acc_index {println!("{:?}", i);};
        for apply in values {
            match apply {
                Apply::Modify {address, basic, code, storage, reset_storage} => {
                    // Print applies
                    let header_size = 60;
                    let codeString = match code {
                        Some(ref code) => {"\"".to_owned() + &hex::encode(&code[..24]) + "...\""},
                        None => {"None".to_string()},
                    };
                    let mut storage = storage.into_iter().peekable();
                    println!("Address: {:?}, basic {:?}, code {}, storage {:?}, reset {:?}",
                            address, basic, codeString, if let Some(_) = storage.peek() {"Not empty"} else {"empty"}, reset_storage);

                    // Get account data
                    let account = self.get_account(address).ok_or_else(|| ProgramError::NotEnoughAccountKeys)?;
                    println!("    {:?}", account);

                    **account.lamports.borrow_mut() = basic.balance.as_u64();
                    let mut data = account.data.borrow_mut();
                    let (mut account_data, rest) = AccountData::unpack(&data)?;

                    let mut current_code_size = match account_data {
                        AccountData::Empty => 0,
                        AccountData::Account{code_size, ..} => code_size as usize,
                    };

                    if let Some(code) = code {
                        if current_code_size != 0 {
                            return Err(ProgramError::AccountAlreadyInitialized);
                        }
                        current_code_size = code.len();
                        data[header_size..header_size+current_code_size].copy_from_slice(&code);
                    }

                    if let Some(_) = storage.peek() {
                        if current_code_size == 0 {
                            return Err(ProgramError::UninitializedAccount);
                        }
                        let mut account_storage = Hamt::new(&mut data[header_size+current_code_size..])?;
                        //if reset_storage {account_storage.reset();}
                        for (key, value) in storage {
                            println!("     {:?} -> {:?}", key, value);
                            account_storage.insert(key.as_fixed_bytes().into(), value.as_fixed_bytes().into());
                        }
                    }

                    let account_data = AccountData::Account{
                        nonce: basic.nonce,
                        address: address,
                        code_size: current_code_size as u64,
                    };

                    println!("   Account data: {:?}", account_data);
                    account_data.pack(&mut data);
                },
                Apply::Delete {address} => {
                },
            }
        };

        for log in logs {
        };

        Ok(())
    }
}

impl<'a> Backend for SolanaBackend<'a> {
    fn gas_price(&self) -> U256 { U256::zero() }
    fn origin(&self) -> H160 { H160::default() }
    fn block_hash(&self, number: U256) -> H256 { H256::default() }
    fn block_number(&self) -> U256 { U256::zero() }
    fn block_coinbase(&self) -> H160 { H160::default() }
    fn block_timestamp(&self) -> U256 { U256::zero() }
    fn block_difficulty(&self) -> U256 { U256::zero() }
    fn block_gas_limit(&self) -> U256 { U256::zero() }
    fn chain_id(&self) -> U256 { U256::zero() }

    fn exists(&self, address: H160) -> bool {
        match self.get_account(address) {
            Some(_) => true,
            None => false,
        }
    }
    fn basic(&self, address: H160) -> Basic { Basic {balance: U256::zero(), nonce: U256::zero() } }
    fn code_hash(&self, address: H160) -> H256 { keccak256_digest(&[]) }
    fn code_size(&self, address: H160) -> usize { 0 }
    fn code(&self, address: H160) -> Vec<u8> { Vec::<u8>::new() }
    fn storage(&self, address: H160, index: H256) -> H256 { H256::default() }
}


#[cfg(test)]
mod test {
    use super::*;
    use solana_sdk::{
        account::Account,
        account_info::{AccountInfo, create_is_signer_account_infos},
        pubkey::Pubkey,
    };
    use evm::executor::StackExecutor;

    pub struct TestContract;
    impl TestContract {
        fn code() -> Vec<u8> {
            hex::decode("608060405234801561001057600080fd5b50336000806101000a81548173ffffffffffffffffffffffffffffffffffffffff021916908373ffffffffffffffffffffffffffffffffffffffff1602179055506000809054906101000a900473ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffff\
                         ffffffffffffffffff16600073ffffffffffffffffffffffffffffffffffffffff167f342827c97908e5e2f71151c08502a66d44b6f758e3ac2f1de95f02eb95f0a73560405160405180910390a361030e806100dc6000396000f3fe60806040526004361061002d5760003560e01c8063893d20e814610087578063a6f9dae1\
                         146100de57610082565b36610082573373ffffffffffffffffffffffffffffffffffffffff167f357b676c439b9e49b4410f8eb8680bee4223724802d8e3fd422e1756f87b475f346040518082815260200191505060405180910390a2005b600080fd5b34801561009357600080fd5b5061009c61012f565b604051808273ff\
                         ffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff16815260200191505060405180910390f35b3480156100ea57600080fd5b5061012d6004803603602081101561010157600080fd5b81019080803573ffffffffffffffffffffffffffffffffffffffff16906020019092\
                         9190505050610158565b005b60008060009054906101000a900473ffffffffffffffffffffffffffffffffffffffff16905090565b6000809054906101000a900473ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff163373ffffffffffffffffffffffffffffffffff\
                         ffffff161461021a576040517f08c379a00000000000000000000000000000000000000000000000000000000081526004018080602001828103825260138152602001807f43616c6c6572206973206e6f74206f776e65720000000000000000000000000081525060200191505060405180910390fd5b8073ffffffffffffff\
                         ffffffffffffffffffffffffff166000809054906101000a900473ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff167f342827c97908e5e2f71151c08502a66d44b6f758e3ac2f1de95f02eb95f0a73560405160405180910390a3806000806101000a81548173ffff\
                         ffffffffffffffffffffffffffffffffffff021916908373ffffffffffffffffffffffffffffffffffffffff1602179055505056fea2646970667358221220b849632806a5977f44b6046c4fe652d5d08e1bbfeec2623ad673961467e58efc64736f6c63430006060033").unwrap()
        }
    
        fn get_owner() -> Vec<u8> {
            let mut v = Vec::new();
            v.extend_from_slice(&0x893d20e8u32.to_be_bytes());
            v
        }
    
        fn change_owner(address: H160) -> Vec<u8> {
            let mut v = Vec::new();
            v.extend_from_slice(&0xa6f9dae1u32.to_be_bytes());
            v.extend_from_slice(&[0u8;12]);
            v.extend_from_slice(&<[u8;20]>::from(address));
            v
        }
    }
    
    pub struct ERC20Contract;
    impl ERC20Contract {
        fn code() -> Vec<u8> {
            hex::decode("608060405234801561001057600080fd5b50604051602080610cce83398101806040528101908080519060200190929190505050806002819055506002546000803373ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff16815260200190815260200160002081905550\
                         50610c3f8061008f6000396000f300608060405260043610610099576000357c0100000000000000000000000000000000000000000000000000000000900463ffffffff16806306fdde031461009e578063095ea7b31461012e57806318160ddd1461019357806323b872dd146101be578063313ce5671461024357806370a0\
                         82311461027457806395d89b41146102cb578063a9059cbb1461035b578063dd62ed3e146103c0575b600080fd5b3480156100aa57600080fd5b506100b3610437565b6040518080602001828103825283818151815260200191508051906020019080838360005b838110156100f35780820151818401526020810190506100\
                         d8565b50505050905090810190601f1680156101205780820380516001836020036101000a031916815260200191505b509250505060405180910390f35b34801561013a57600080fd5b50610179600480360381019080803573ffffffffffffffffffffffffffffffffffffffff169060200190929190803590602001909291\
                         90505050610470565b604051808215151515815260200191505060405180910390f35b34801561019f57600080fd5b506101a8610562565b6040518082815260200191505060405180910390f35b3480156101ca57600080fd5b50610229600480360381019080803573ffffffffffffffffffffffffffffffffffffffff1690\
                         60200190929190803573ffffffffffffffffffffffffffffffffffffffff1690602001909291908035906020019092919050505061056c565b604051808215151515815260200191505060405180910390f35b34801561024f57600080fd5b506102586108eb565b604051808260ff1660ff1681526020019150506040518091\
                         0390f35b34801561028057600080fd5b506102b5600480360381019080803573ffffffffffffffffffffffffffffffffffffffff1690602001909291905050506108f0565b6040518082815260200191505060405180910390f35b3480156102d757600080fd5b506102e0610938565b60405180806020018281038252838181\
                         51815260200191508051906020019080838360005b83811015610320578082015181840152602081019050610305565b50505050905090810190601f16801561034d5780820380516001836020036101000a031916815260200191505b509250505060405180910390f35b34801561036757600080fd5b506103a66004803603\
                         81019080803573ffffffffffffffffffffffffffffffffffffffff16906020019092919080359060200190929190505050610971565b604051808215151515815260200191505060405180910390f35b3480156103cc57600080fd5b50610421600480360381019080803573ffffffffffffffffffffffffffffffffffffffff\
                         169060200190929190803573ffffffffffffffffffffffffffffffffffffffff169060200190929190505050610b55565b6040518082815260200191505060405180910390f35b6040805190810160405280600a81526020017f455243323042617369630000000000000000000000000000000000000000000081525081565b\
                         600081600160003373ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff16815260200190815260200160002060008573ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff16815260200190815260200160002081\
                         9055508273ffffffffffffffffffffffffffffffffffffffff163373ffffffffffffffffffffffffffffffffffffffff167f8c5be1e5ebec7d5bd14f71427d1e84f3dd0314c0f7b2291e5b200ac8c7c3b925846040518082815260200191505060405180910390a36001905092915050565b6000600254905090565b60008060\
                         008573ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020016000205482111515156105bb57600080fd5b600160008573ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020\
                         0190815260200160002060003373ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff16815260200190815260200160002054821115151561064657600080fd5b610697826000808773ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffff\
                         ffffffffffffffffff16815260200190815260200160002054610bdc90919063ffffffff16565b6000808673ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020016000208190555061076882600160008773ffffffffffffffffffffffffff\
                         ffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff16815260200190815260200160002060003373ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff16815260200190815260200160002054610bdc90919063ffffffff16565b600160008673ffff\
                         ffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff16815260200190815260200160002060003373ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff168152602001908152602001600020819055506108398260008086\
                         73ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff16815260200190815260200160002054610bf590919063ffffffff16565b6000808573ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081\
                         52602001600020819055508273ffffffffffffffffffffffffffffffffffffffff168473ffffffffffffffffffffffffffffffffffffffff167fddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef846040518082815260200191505060405180910390a3600190509392505050565b601281565b\
                         60008060008373ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff168152602001908152602001600020549050919050565b6040805190810160405280600381526020017f42534300000000000000000000000000000000000000000000000000000000008152508156\
                         5b60008060003373ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020016000205482111515156109c057600080fd5b610a11826000803373ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffff\
                         ffffff16815260200190815260200160002054610bdc90919063ffffffff16565b6000803373ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff16815260200190815260200160002081905550610aa4826000808673ffffffffffffffffffffffffffffffffffffffff\
                         1673ffffffffffffffffffffffffffffffffffffffff16815260200190815260200160002054610bf590919063ffffffff16565b6000808573ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff168152602001908152602001600020819055508273ffffffffffffffff\
                         ffffffffffffffffffffffff163373ffffffffffffffffffffffffffffffffffffffff167fddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef846040518082815260200191505060405180910390a36001905092915050565b6000600160008473ffffffffffffffffffffffffffffffffffffff\
                         ff1673ffffffffffffffffffffffffffffffffffffffff16815260200190815260200160002060008373ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff16815260200190815260200160002054905092915050565b6000828211151515610bea57fe5b818303905092\
                         915050565b6000808284019050838110151515610c0957fe5b80915050929150505600a165627a7a72305820f4d60144ea79e518441446668eb613530c0faffbf3abddad6bce0dd8de29e8f4002900000000000000000000000000000000000000000000000000000000000186a0").unwrap()
        }
    
        fn transfer() -> Vec<u8> {
            hex::decode("a9059cbb00000000000000000000000002033f13228cce65cba457d62b31df9808717ee000000000000000000000000000000000000000000000000000000000000004d2").unwrap()
        }
    }

    #[test]
    fn test_solana_backend() {
        let owner = Pubkey::new_rand();
        let mut accounts = Vec::new();
        for i in 0..8 {
            accounts.push( (
                    Pubkey::new_rand(), i,
                    Account::new(((i+2)*1000) as u64, 10*1024, &owner)
                ) );
        }

        let mut infos = Vec::new();
        for acc in &mut accounts {
            infos.push(AccountInfo::from((&acc.0, acc.1==0, &mut acc.2)));
        }

        let mut backend = SolanaBackend::new(&infos[1..]);

        let config = evm::Config::istanbul();
        let mut executor = StackExecutor::new(&backend, usize::max_value(), &config);

        assert_eq!(backend.exists(solidity_address(&owner)), false);
        assert_eq!(backend.exists(solidity_address(infos[1].key)), true);

        let creator = solidity_address(infos[1].key);
        println!("Creator: {:?}", creator);
        executor.deposit(creator, U256::exp10(18));

        let contract = executor.create_address(CreateScheme::Legacy{caller: creator});
        let exit_reason = executor.transact_create(creator, U256::zero(), TestContract::code(), usize::max_value());
        println!("Create contract {:?}: {:?}", contract, exit_reason);

        let (applies, logs) = executor.deconstruct();

        backend.add_account(contract, &infos[0]);
        let apply_result = backend.apply(applies, logs, false);
        println!("Apply result: {:?}", apply_result);

        println!();
        let mut executor = StackExecutor::new(&backend, usize::max_value(), &config);

        let (exit_reason, result) = executor.transact_call(
                creator, contract, U256::zero(), TestContract::get_owner(), usize::max_value());
        println!("Call: {:?}, {}", exit_reason, hex::encode(&result));

        let (applies, logs) = executor.deconstruct();
        backend.apply(applies, logs, false);
        

        println!();
        for acc in &accounts {
            println!("{:x?}", acc);
        }
    }
}
