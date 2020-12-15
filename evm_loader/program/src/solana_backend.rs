use evm::{
    backend::{Basic, Backend, ApplyBackend, Apply, Log},
    CreateScheme, Capture, Transfer, ExitReason
};
use core::convert::Infallible;
use primitive_types::{H160, H256, U256};
use sha3::{Digest, Keccak256};
use solana_sdk::{
    account_info::AccountInfo,
    pubkey::Pubkey,
    program_error::ProgramError,
    info,
    instruction
};
use std::cell::RefCell;

use crate::solidity_account::SolidityAccount;
use crate::account_data::AccountData;
use solana_sdk::program::invoke;
use solana_sdk::program::invoke_signed;
use std::convert::TryInto;
use std::str::FromStr;

fn keccak256_digest(data: &[u8]) -> H256 {
    H256::from_slice(Keccak256::digest(&data).as_slice())
}

pub fn solidity_address<'a>(key: &Pubkey) -> H160 {
    H256::from_slice(key.as_ref()).into()
}

fn U256_to_H256(value: U256) -> H256 {
    let mut v = vec![0u8; 32];
    value.to_big_endian(&mut v);
    H256::from_slice(&v)
}

pub struct SolanaBackend<'a> {
    accounts: Vec<SolidityAccount<'a>>,
    aliases: RefCell<Vec<(H160, usize)>>,
}

impl<'a> SolanaBackend<'a> {
    pub fn new(program_id: &Pubkey, accountInfos: &'a [AccountInfo<'a>]) -> Result<Self,ProgramError> {
        info!("backend::new");
        let mut accounts = Vec::with_capacity(accountInfos.len());
        let mut aliases = Vec::with_capacity(accountInfos.len());

        for (i, account) in (&accountInfos).iter().enumerate() {
            info!(&i.to_string());
            let sol_account = if account.owner == program_id {SolidityAccount::new(account)?}
                    else {SolidityAccount::foreign(account)?};
            //println!(" ==> sol_account: {:?}", sol_account);
            aliases.push((sol_account.get_address(), i));
            accounts.push(sol_account);
        };
        info!("Accounts was read");
        aliases.sort_by_key(|v| v.0);
        Ok(Self {accounts: accounts, aliases: RefCell::new(aliases)})
    }

    pub fn get_address_by_index(&self, index: usize) -> H160 {
        self.accounts[index].get_address()
    }

    pub fn add_alias(&self, address: &H160, pubkey: &Pubkey) {
        info!(&("Add alias ".to_owned() + &address.to_string() + " for " + &pubkey.to_string()));
        for (i, account) in (&self.accounts).iter().enumerate() {
            if account.accountInfo.key == pubkey {
                let mut aliases = self.aliases.borrow_mut();
                aliases.push((*address, i));
                aliases.sort_by_key(|v| v.0);
                return;
            }
        }
    }

    fn find_account(&self, address: H160) -> Option<usize> {
        let aliases = self.aliases.borrow();
        match aliases.binary_search_by_key(&address, |v| v.0) {
            Ok(pos) => {
                info!(&("Found account for ".to_owned() + &address.to_string() + " on position " + &pos.to_string()));
                Some(aliases[pos].1)
            },
            Err(_) => {
                info!(&("Not found account for ".to_owned() + &address.to_string()));
                None
            },
        }
    }

    fn get_account(&self, address: H160) -> Option<&SolidityAccount<'a>> {
        self.find_account(address).map(|pos| &self.accounts[pos])
    }

    fn get_account_mut(&mut self, address: H160) -> Option<&mut SolidityAccount<'a>> {
        if let Some(pos) = self.find_account(address) {
            Some(&mut self.accounts[pos])
        } else {None}
    }

    fn is_solana_address(&self, code_address: &H160) -> bool {
        return code_address.to_string() == "0xff00…0000";
    }

    pub fn apply<A, I, L>(&mut self, values: A, logs: L, delete_empty: bool) -> Result<(), ProgramError>
            where
                A: IntoIterator<Item=Apply<I>>,
                I: IntoIterator<Item=(H256, H256)>,
                L: IntoIterator<Item=Log>,
    {
        for apply in values {
            match apply {
                Apply::Modify {address, basic, code, storage, reset_storage} => {
                    if self.is_solana_address(&address) {
                        continue;
                    }
                    let account = self.get_account_mut(address).ok_or_else(|| ProgramError::NotEnoughAccountKeys)?;
                    account.update(address, basic.nonce, basic.balance.as_u64(), &code, storage, reset_storage)?;
                },
                Apply::Delete {address} => {},
            }
        };

        //for log in logs {};

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
    fn basic(&self, address: H160) -> Basic {
        match self.get_account(address) {
            None => Basic{balance: U256::zero(), nonce: U256::zero()},
            Some(acc) => Basic{
                balance: (**acc.accountInfo.lamports.borrow()).into(),
                nonce: if let AccountData::Account{nonce, ..} = acc.accountData {nonce} else {U256::zero()},
            },
        }
    }
    fn code_hash(&self, address: H160) -> H256 {
        self.get_account(address).map_or_else(
                || keccak256_digest(&[]), 
                |acc| acc.code(|d| keccak256_digest(d))
            )
    }
    fn code_size(&self, address: H160) -> usize {
        self.get_account(address).map_or_else(|| 0, |acc| acc.code(|d| d.len()))
    }
    fn code(&self, address: H160) -> Vec<u8> {
        let code = self.get_account(address).map_or_else(
                || Vec::new(),
                |acc| acc.code(|d| d.into())
            );
        info!(&("Get code for ".to_owned() + &address.to_string() +
                " " + &hex::encode(&code[..])));
        code
    }
    fn storage(&self, address: H160, index: H256) -> H256 {
        let result = match self.get_account(address) {
            None => H256::default(),
            Some(acc) => {
                let index = index.as_fixed_bytes().into();
                let value = acc.storage(|storage| storage.find(index)).unwrap_or_default();
                if let Some(v) = value {U256_to_H256(v)} else {H256::default()}
            },
        };
        info!(&("Storage ".to_owned() + &address.to_string() + " : " + &index.to_string() + " = " +
                &result.to_string()));
        result
    }

    fn create(&self, scheme: &CreateScheme, address: &H160) {
        let account = if let CreateScheme::Create2{salt,..} = scheme
                {Pubkey::new(&salt.to_fixed_bytes())} else {Pubkey::default()};
        //println!("Create new account: {:x?} -> {:x?} // {}", scheme, address, account);
        self.add_alias(address, &account);
    }

    fn call_inner(&self,
        code_address: H160,
        _transfer: Option<Transfer>,
        _input: Vec<u8>,
        _target_gas: Option<usize>,
        _is_static: bool,
        _take_l64: bool,
        _take_stipend: bool,
    ) -> Option<Capture<(ExitReason, Vec<u8>), Infallible>> {
        if (!self.is_solana_address(&code_address)) {
            return None;
        }

        let (program_id_len, rest) = _input.split_at(2);
        let program_id_len = program_id_len
            .try_into()
            .ok()
            .map(u16::from_be_bytes)
            .unwrap();
        let (program_id_str, rest) = rest.split_at(program_id_len as usize);
        let program_id = Pubkey::new(program_id_str);

        let mut accountMetas = Vec::new();
        let mut accountInfos = Vec::new();
        let (accs_len, rest) = rest.split_at(2);
        let accs_len = accs_len
            .try_into()
            .ok()
            .map(u16::from_be_bytes)
            .unwrap();
        let mut sl = rest;
        for i in 0..accs_len {
            let (needs_translate, rest) = rest.split_at(1);
            let needs_translate = needs_translate[0] != 0;
            let mut acc_len = 32;
            if needs_translate { acc_len = 20; }

            let (acc, rest) = sl.split_at(acc_len);

            let (is_signer, rest) = rest.split_at(1);
            let is_signer = is_signer[0] != 0;

            let (is_writable, rest) = rest.split_at(1);
            let is_writable = is_writable[0] != 0;

            sl = rest;

            if (needs_translate) {
                let acc_id = H160::from_slice(acc);
                let acc_opt = self.get_account(acc_id);
                if acc_opt.is_none() {
                    return Some(Capture::Exit((ExitReason::Error(evm::ExitError::InvalidRange), Vec::new())));
                }
                let acc = acc_opt.unwrap().accountInfo.clone();
                accountMetas.push(instruction::AccountMeta { 
                    pubkey: acc.key.clone(), 
                    is_signer: is_signer,
                    is_writable: is_writable });
                accountInfos.push(acc);
            } else {
                let key = Pubkey::new(acc);
                accountMetas.push(instruction::AccountMeta { 
                    pubkey: key,
                    is_signer: is_signer,
                    is_writable: is_writable });
            }
        }

        let (data_len, rest) = sl.split_at(2);
        let data_len = data_len
            .try_into()
            .ok()
            .map(u16::from_be_bytes)
            .unwrap();

        let (data, rest) = rest.split_at(data_len as usize);

        let ix = instruction::Instruction {
            program_id,
            accounts: accountMetas,
            data: data.to_vec()
        };
        invoke(
            &ix,
            &accountInfos,
        );
        return Some(Capture::Exit((ExitReason::Succeed(evm::ExitSucceed::Stopped), Vec::new())));
    }
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
        fn wrapper_code() -> Vec<u8> {
            hex::decode("608060405273ff000000000000000000000000000000000000006000806101000a81548173ffffffffffffffffffffffffffffffffffffffff021916908373ffffffffffffffffffffffffffffffffffffffff16021790555034801561006457600080fd5b50610ca0806100746000396000f3fe608060405234801561001057600080fd5b50600436106100365760003560e01c806354d5db4c1461003b578063fa432d5d14610057575b600080fd5b61005560048036036100509190810190610657565b610073565b005b610071600480360361006c91908101906105b0565b610210565b005b600060019050606060036040519080825280602002602001820160405280156100b657816020015b6100a36104ef565b81526020019060019003908161009b5790505b50905060405180608001604052806000151581526020016000809054906101000a900473ffffffffffffffffffffffffffffffffffffffff166040516020016100ff91906108f1565b6040516020818303038152906040528152602001600115158152602001600015158152508160008151811061013057fe5b602002602001018190525060405180608001604052806001151581526020013060405160200161016091906108f1565b6040516020818303038152906040528152602001600115158152602001600015158152508160018151811061019157fe5b60200260200101819052506040518060800160405280861515815260200185815260200160001515815260200160011515815250816002815181106101d257fe5b60200260200101819052506102088183856040516020016101f492919061094f565b60405160208183030381529060405261038e565b505050505050565b60008090506060600360405190808252806020026020018201604052801561025257816020015b61023f6104ef565b8152602001906001900390816102375790505b50905060405180608001604052806000151581526020016000809054906101000a900473ffffffffffffffffffffffffffffffffffffffff1660405160200161029b91906108f1565b604051602081830303815290604052815260200160011515815260200160001515815250816000815181106102cc57fe5b602002602001018190525060405180608001604052808815158152602001878152602001600115158152602001600015158152508160018151811061030d57fe5b602002602001018190525060405180608001604052808615158152602001858152602001600015158152602001600115158152508160028151811061034e57fe5b6020026020010181905250610384818385604051602001610370929190610923565b60405160208183030381529060405261038e565b5050505050505050565b6060600060606040518060600160405280602b8152602001610c33602b9139905060608186866040516024016103c69392919061097b565b6040516020818303038152906040527ff6fb1cc3000000000000000000000000000000000000000000000000000000007bffffffffffffffffffffffffffffffffffffffffffffffffffffffff19166020820180517bffffffffffffffffffffffffffffffffffffffffffffffffffffffff8381831617835250505050905060606000809054906101000a900473ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff168260405161048d919061090c565b6000604051808303816000865af19150503d80600081146104ca576040519150601f19603f3d011682016040523d82523d6000602084013e6104cf565b606091505b508092508195505050836104e257600080fd5b8094505050505092915050565b6040518060800160405280600015158152602001606081526020016000151581526020016000151581525090565b60008135905061052c81610bed565b92915050565b600082601f83011261054357600080fd5b8135610556610551826109f4565b6109c7565b9150808252602083016020830185838301111561057257600080fd5b61057d838284610b21565b50505092915050565b60008135905061059581610c04565b92915050565b6000813590506105aa81610c1b565b92915050565b600080600080600060a086880312156105c857600080fd5b60006105d68882890161051d565b955050602086013567ffffffffffffffff8111156105f357600080fd5b6105ff88828901610532565b94505060406106108882890161051d565b935050606086013567ffffffffffffffff81111561062d57600080fd5b61063988828901610532565b925050608061064a88828901610586565b9150509295509295909350565b60008060006060848603121561066c57600080fd5b600061067a8682870161051d565b935050602084013567ffffffffffffffff81111561069757600080fd5b6106a386828701610532565b92505060406106b48682870161059b565b9150509250925092565b60006106ca8383610849565b905092915050565b6106e36106de82610ab8565b610b63565b82525050565b60006106f482610a30565b6106fe8185610a69565b93508360208202850161071085610a20565b8060005b8581101561074c578484038952815161072d85826106be565b945061073883610a5c565b925060208a01995050600181019050610714565b50829750879550505050505092915050565b61076781610aca565b82525050565b600061077882610a46565b6107828185610a8b565b9350610792818560208601610b30565b61079b81610bb5565b840191505092915050565b60006107b182610a46565b6107bb8185610a9c565b93506107cb818560208601610b30565b80840191505092915050565b60006107e282610a3b565b6107ec8185610a7a565b93506107fc818560208601610b30565b61080581610bb5565b840191505092915050565b600061081b82610a51565b6108258185610aa7565b9350610835818560208601610b30565b61083e81610bb5565b840191505092915050565b6000608083016000830151610861600086018261075e565b506020830151848203602086015261087982826107d7565b915050604083015161088e604086018261075e565b5060608301516108a1606086018261075e565b508091505092915050565b6108bd6108b882610af6565b610b87565b82525050565b6108d46108cf82610b00565b610b91565b82525050565b6108eb6108e682610b14565b610ba3565b82525050565b60006108fd82846106d2565b60148201915081905092915050565b600061091882846107a6565b915081905092915050565b600061092f82856108da565b60018201915061093f82846108ac565b6020820191508190509392505050565b600061095b82856108da565b60018201915061096b82846108c3565b6008820191508190509392505050565b600060608201905081810360008301526109958186610810565b905081810360208301526109a981856106e9565b905081810360408301526109bd818461076d565b9050949350505050565b6000604051905081810181811067ffffffffffffffff821117156109ea57600080fd5b8060405250919050565b600067ffffffffffffffff821115610a0b57600080fd5b601f19601f8301169050602081019050919050565b6000819050602082019050919050565b600081519050919050565b600081519050919050565b600081519050919050565b600081519050919050565b6000602082019050919050565b600082825260208201905092915050565b600082825260208201905092915050565b600082825260208201905092915050565b600081905092915050565b600082825260208201905092915050565b6000610ac382610ad6565b9050919050565b60008115159050919050565b600073ffffffffffffffffffffffffffffffffffffffff82169050919050565b6000819050919050565b600067ffffffffffffffff82169050919050565b600060ff82169050919050565b82818337600083830152505050565b60005b83811015610b4e578082015181840152602081019050610b33565b83811115610b5d576000848401525b50505050565b6000610b6e82610b75565b9050919050565b6000610b8082610be0565b9050919050565b6000819050919050565b6000610b9c82610bc6565b9050919050565b6000610bae82610bd3565b9050919050565b6000601f19601f8301169050919050565b60008160c01b9050919050565b60008160f81b9050919050565b60008160601b9050919050565b610bf681610aca565b8114610c0157600080fd5b50565b610c0d81610af6565b8114610c1857600080fd5b50565b610c2481610b00565b8114610c2f57600080fd5b5056fe546f6b656e6b65675166655a79694e77414a624e62474b5046584357754276663953733632335651354441a365627a7a72315820e5121293a83e25a54f9242231e22c734eaf2d099e0cf50b0b2e55ed664f1b5626c6578706572696d656e74616cf564736f6c63430005110040").unwrap()
        }

        fn code() -> Vec<u8> {
            hex::decode("608060405234801561001057600080fd5b5061020e806100206000396000f3fe608060405234801561001057600080fd5b50600436106100365760003560e01c80633071fbec1461003b578063ed88c68e14610045575b600080fd5b61004361004f565b005b61004d610128565b005b60003090508073ffffffffffffffffffffffffffffffffffffffff1663fa432d5d60018060056040518463ffffffff1660e01b81526004018084151515158152602001806020018415151515815260200180602001848152602001838103835260148152602001806c02000000000000000000000000815250602001838103825260148152602001806c0100000000000000000000000081525060200195505050505050600060405180830381600087803b15801561010d57600080fd5b505af1158015610121573d6000803e3d6000fd5b5050505050565b60003090508073ffffffffffffffffffffffffffffffffffffffff166354d5db4c600160056040518363ffffffff1660e01b81526004018083151515158152602001806020018367ffffffffffffffff168152602001828103825260148152602001806c010000000000000000000000008152506020019350505050600060405180830381600087803b1580156101be57600080fd5b505af11580156101d2573d6000803e3d6000fd5b505050505056fea265627a7a72315820eb79b1881e4fd439c4b5bd20606fd9854b8461ee11d9fc7d62d91f3f0ffdee5464736f6c63430005110032").unwrap()
        }

        fn donate() -> Vec<u8> {
            hex::decode("ed88c68e").unwrap();
        }

        fn donateFrom() -> Vec<u8> {
            hex::decode("3071fbec").unwrap();
        }
    }

    #[test]
    fn test_solana_backend() -> Result<(), ProgramError> {
        let owner = Pubkey::new_rand();
        let mut accounts = Vec::new();

        for i in 0..4 {
            accounts.push( (
                    Pubkey::new_rand(), i == 0,
                    Account::new(((i+2)*1000) as u64, 10*1024, &owner)
                ) );
        }
        accounts.push((Pubkey::new_rand(), false, Account::new(1234u64, 0, &owner)));
        accounts.push((Pubkey::new_rand(), false, Account::new(5423u64, 1024, &Pubkey::new_rand())));
        accounts.push((Pubkey::new_rand(), false, Account::new(1234u64, 0, &Pubkey::new_rand())));

        for acc in &accounts {println!("{:x?}", acc);};

        let mut infos = Vec::new();
        for acc in &mut accounts {
            infos.push(AccountInfo::from((&acc.0, acc.1, &mut acc.2)));
        }

        let mut backend = SolanaBackend::new(&owner, &infos[..]).unwrap();

        let config = evm::Config::istanbul();
        let mut executor = StackExecutor::new(&backend, usize::max_value(), &config);

        assert_eq!(backend.exists(solidity_address(&owner)), false);
        assert_eq!(backend.exists(solidity_address(infos[1].key)), true);

        let creator = solidity_address(infos[1].key);
        println!("Creator: {:?}", creator);
        executor.deposit(creator, U256::exp10(18));

        let contract = executor.create_address(CreateScheme::Create2{caller: creator, code_hash: keccak256_digest(&TestContract::code()), salt: infos[0].key.to_bytes().into()});
        let exit_reason = executor.transact_create2(creator, U256::zero(), TestContract::code(), infos[0].key.to_bytes().into(), usize::max_value());
        println!("Create contract {:?}: {:?}", contract, exit_reason);

        let (applies, logs) = executor.deconstruct();

//        backend.add_account(contract, &infos[0]);
        let apply_result = backend.apply(applies, logs, false);
        println!("Apply result: {:?}", apply_result);

        println!();
//        let mut backend = SolanaBackend::new(&infos).unwrap();
        let mut executor = StackExecutor::new(&backend, usize::max_value(), &config);
        println!("======================================");
        println!("Contract: {:x}", contract);
        println!("{:x?}", backend.exists(contract));
        println!("{:x}", backend.code_size(contract));
        println!("code_hash {:x}", backend.code_hash(contract));
        println!("code: {:x?}", hex::encode(backend.code(contract)));
        println!("storage value: {:x}", backend.storage(contract, H256::default()));
        println!();

        println!("Creator: {:x}", creator);
        println!("code_size: {:x}", backend.code_size(creator));
        println!("code_hash: {:x}", backend.code_hash(creator));
        println!("code: {:x?}", hex::encode(backend.code(creator)));

        println!("Missing account code_size: {:x}", backend.code_size(H160::zero()));
        println!("Code_hash: {:x}", backend.code_hash(H160::zero()));
        println!("storage value: {:x}", backend.storage(H160::zero(), H256::default()));

        let (exit_reason, result) = executor.transact_call(
                creator, contract, U256::zero(), TestContract::get_owner(), usize::max_value());
        println!("Call: {:?}, {}", exit_reason, hex::encode(&result));

        let (applies, logs) = executor.deconstruct();
        backend.apply(applies, logs, false)?;
        

/*        println!();
        for acc in &accounts {
            println!("{:x?}", acc);
        }*/
        Ok(())
    }

    #[test]
    fn test_erc20_wrapper() -> Result<(), ProgramError> {
        let owner = Pubkey::new_rand();
        let mut accounts = Vec::new();

        for i in 0..4 {
            accounts.push( (
                    Pubkey::new_rand(), i == 0,
                    Account::new(((i+2)*1000) as u64, 10*1024, &owner)
                ) );
        }
        accounts.push((Pubkey::new_rand(), false, Account::new(1234u64, 0, &owner)));
        accounts.push((Pubkey::new_rand(), false, Account::new(5423u64, 1024, &Pubkey::new_rand())));
        accounts.push((Pubkey::new_rand(), false, Account::new(1234u64, 0, &Pubkey::new_rand())));

        for acc in &accounts {println!("{:x?}", acc);};

        let mut infos = Vec::new();
        for acc in &mut accounts {
            infos.push(AccountInfo::from((&acc.0, acc.1, &mut acc.2)));
        }

        let mut backend = SolanaBackend::new(&owner, &infos[..]).unwrap();

        let config = evm::Config::istanbul();
        let mut executor = StackExecutor::new(&backend, usize::max_value(), &config);

        assert_eq!(backend.exists(solidity_address(&owner)), false);
        assert_eq!(backend.exists(solidity_address(infos[1].key)), true);

        let creator = solidity_address(infos[1].key);
        println!("Creator: {:?}", creator);
        executor.deposit(creator, U256::exp10(18));

        let contract = executor.create_address(CreateScheme::Create2{caller: creator, code_hash: keccak256_digest(&ERC20Contract::wrapper_code()), salt: infos[0].key.to_bytes().into()});
        let exit_reason = executor.transact_create2(creator, U256::zero(), ERC20Contract::wrapper_code(), infos[0].key.to_bytes().into(), usize::max_value());
        println!("Create contract {:?}: {:?}", contract, exit_reason);

        contract = executor.create_address(CreateScheme::Create2{caller: creator, code_hash: keccak256_digest(&ERC20Contract::code()), salt: infos[0].key.to_bytes().into()});
        exit_reason = executor.transact_create2(creator, U256::zero(), ERC20Contract::code(), infos[0].key.to_bytes().into(), usize::max_value());
        println!("Create contract {:?}: {:?}", contract, exit_reason);

        let (applies, logs) = executor.deconstruct();

//        backend.add_account(contract, &infos[0]);
        let apply_result = backend.apply(applies, logs, false);
        println!("Apply result: {:?}", apply_result);

        println!();
//        let mut backend = SolanaBackend::new(&infos).unwrap();
        let mut executor = StackExecutor::new(&backend, usize::max_value(), &config);
        println!("======================================");
        println!("Contract: {:x}", contract);
        println!("{:x?}", backend.exists(contract));
        println!("{:x}", backend.code_size(contract));
        println!("code_hash {:x}", backend.code_hash(contract));
        println!("code: {:x?}", hex::encode(backend.code(contract)));
        println!("storage value: {:x}", backend.storage(contract, H256::default()));
        println!();

        println!("Creator: {:x}", creator);
        println!("code_size: {:x}", backend.code_size(creator));
        println!("code_hash: {:x}", backend.code_hash(creator));
        println!("code: {:x?}", hex::encode(backend.code(creator)));

        println!("Missing account code_size: {:x}", backend.code_size(H160::zero()));
        println!("Code_hash: {:x}", backend.code_hash(H160::zero()));
        println!("storage value: {:x}", backend.storage(H160::zero(), H256::default()));

        let (exit_reason, result) = executor.transact_call(
                creator, contract, U256::zero(), ERC20Contract::donate(), usize::max_value());
        println!("Call: {:?}, {}", exit_reason, hex::encode(&result));

        let (exit_reason, result) = executor.transact_call(
                creator, contract, U256::zero(), ERC20Contract::donateFrom(), usize::max_value());
        println!("Call: {:?}, {}", exit_reason, hex::encode(&result));

        let (applies, logs) = executor.deconstruct();
        backend.apply(applies, logs, false)?;
        

/*        println!();
        for acc in &accounts {
            println!("{:x?}", acc);
        }*/
        Ok(())
    }
}
