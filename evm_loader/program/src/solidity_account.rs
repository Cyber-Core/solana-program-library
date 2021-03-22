use crate::account_data::AccountData;
use solana_sdk::program_error::ProgramError;
// use crate::constatns::ProgramError;
use crate::hamt::Hamt;
use solana_sdk::account_info::AccountInfo;
use solana_sdk::pubkey::Pubkey;
use primitive_types::{H160, H256, U256};
use std::cell::RefCell;
use std::convert::TryInto;
use std::rc::Rc;

#[derive(Debug, Clone)]
pub enum Data<'a> {
    Program(Rc<RefCell<&'a mut [u8]>>),
    Emulator(RefCell<Vec<u8>>),
}

#[derive(Debug, Clone)]
pub struct SolidityAccount<'a> {
    //pub key: H160,
    pub account_data: AccountData,
    pub solana_address: Pubkey,
    pub data: Data<'a>,
    pub lamports: u64,
    pub updated: bool,
}

impl<'a> SolidityAccount<'a> {
    pub fn new(solana_address: Pubkey, data: Rc<RefCell<&'a mut [u8]>>, lamports: u64) -> Result<Self, ProgramError> {
        debug_print!("  SolidityAccount::new");
        let data_b = data.borrow();
        debug_print!(&("  Get data with length ".to_owned() + &data_b.len().to_string()));
        let (account_data, _) = AccountData::unpack(&data_b)?;
        Ok(Self{account_data, solana_address, data: Data::Program(data.clone()), lamports, updated: false})
    }

    pub fn new_emulator(solana_address: Pubkey, data: Vec<u8>, lamports: u64) -> Result<Self, u8> {
        eprintln!("  SolidityAccount::new");
        eprintln!("  Get data with length {}", data.len());
        let (account_data, _) = AccountData::unpack(&data.as_slice()).unwrap();
        eprintln!("Unpack: {} {}", &account_data.trx_count, &lamports);
        Ok(Self{account_data, solana_address, data: Data::Emulator(RefCell::new(data)), lamports, updated: false})
    }

    pub fn get_ether(&self) -> H160 {self.account_data.ether}

    pub fn get_nonce(&self) -> u64 {self.account_data.trx_count}

    pub fn code<U, F>(&self, f: F) -> U
    where F: FnOnce(&[u8]) -> U {
        /*if let AccountData::Account{code_size,..} = self.account_data {
            if code_size > 0 {
                let data = self.account_info.data.borrow();
                let offset = AccountData::size();
                return f(&data[offset..offset+code_size as usize])
            }
        }*/
        if self.account_data.code_size > 0 {
            match &self.data {
                Data::Program(data) => {
                    let data = data.borrow();
                    let offset = AccountData::SIZE;
                    let code_size = self.account_data.code_size as usize;
                    f(&data[offset..offset + code_size])
                }, 
                Data::Emulator(data) => {
                    let data = data.borrow();
                    let offset = AccountData::SIZE;
                    let code_size = self.account_data.code_size as usize;
                    f(&data[offset..offset + code_size])
                },
            }
        } else {
            f(&[])
        }
    }

    pub fn storage<U, F>(&self, f: F) -> Result<U, ProgramError>
    where F: FnOnce(&mut Hamt) -> U {
        /*if let AccountData::Account{code_size,..} = self.account_data {
            if code_size > 0 {
                let mut data = self.account_info.data.borrow_mut();
                debug_print!("Storage data borrowed");
                let offset = AccountData::size() + code_size as usize;
                let mut hamt = Hamt::new(&mut data[offset..], false)?;
                return Ok(f(&mut hamt));
            }
        }
        Err(ProgramError::UninitializedAccount)*/
        if self.account_data.code_size > 0 {
            match &self.data {
                Data::Program(p_data) => {
                    let mut data = (**p_data).borrow_mut();
                    debug_print!("Storage data borrowed");
                    let code_size = self.account_data.code_size as usize;
                    let offset = AccountData::SIZE + code_size;
                    let mut hamt = Hamt::new(&mut data[offset..], false)?;
                    Ok(f(&mut hamt))
                }, 
                Data::Emulator(e_data) => {
                    let mut data = e_data.borrow_mut();
                    debug_print!("Storage data borrowed");
                    let code_size = self.account_data.code_size as usize;
                    let offset = AccountData::SIZE + code_size;
                    let mut hamt = Hamt::new(&mut data[offset..], false)?;
                    Ok(f(&mut hamt))
                },
            }
        } else {
            Err(ProgramError::UninitializedAccount)
        }
    }

    pub fn update<I>(
        &mut self,
        account_info: &'a AccountInfo<'a>,
        solidity_address: H160,
        nonce: U256,
        lamports: u64,
        code: &Option<Vec<u8>>,
        storage_items: I,
        reset_storage: bool,
    ) -> Result<(), ProgramError>
    where I: IntoIterator<Item = (H256, H256)> 
    {
        println!("Update: {}, {}, {}, {:?} for {:?}", solidity_address, nonce, lamports, if let Some(_) = code {"Exist"} else {"Empty"}, self);
        let mut data = (*account_info.data).borrow_mut();
        **(*account_info.lamports).borrow_mut() = lamports;

        /*let mut current_code_size = match self.account_data {
            AccountData::Empty => 0,
            AccountData::Foreign => 0,
            AccountData::Account{code_size, ..} => code_size as usize,
        };*/
        self.account_data.trx_count = nonce.as_u64();
        if let Some(code) = code {
            if self.account_data.code_size != 0 {
                return Err(ProgramError::AccountAlreadyInitialized);
            };
            self.account_data.code_size = code.len().try_into().map_err(|_| ProgramError::AccountDataTooSmall)?;
            debug_print!("Write code");
            data[AccountData::SIZE..AccountData::SIZE + code.len()].copy_from_slice(&code);
            debug_print!("Code written");
        }

        debug_print!("Write account data");
        self.account_data.pack(&mut data)?;

        let mut storage_iter = storage_items.into_iter().peekable();
        let exist_items = if let Some(_) = storage_iter.peek() {true} else {false};
        if reset_storage || exist_items {
            debug_print!("Update storage");
            let code_size = self.account_data.code_size as usize;
            if code_size == 0 {return Err(ProgramError::UninitializedAccount);};

            let mut storage = Hamt::new(&mut data[AccountData::SIZE + code_size..], reset_storage)?;
            debug_print!("Storage initialized");
            for (key, value) in storage_iter {
                debug_print!(&("Storage value: ".to_owned() + &key.to_string() + " = " + &value.to_string()));
                storage.insert(key.as_fixed_bytes().into(), value.as_fixed_bytes().into())?;
            }
        }

        debug_print!("Account updated");
        Ok(())
    }
}
