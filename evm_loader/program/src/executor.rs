use std::collections::{BTreeMap, BTreeSet};
use std::convert::Infallible;
use std::rc::Rc;

use primitive_types::{H160, H256, U256};
use sha3::{Digest, Keccak256};

use crate::solana_backend::SolanaBackend;
use crate::executor_state::{ StackState, ExecutorState, ExecutorMetadata};

use evm::{backend::Backend, Capture, ExitError, ExitReason, ExitSucceed, ExitFatal, Handler};


macro_rules! try_or_fail {
    ( $e:expr ) => {
        match $e {
            Ok(v) => v,
            Err(e) => return e.into(),
        }
    }
}

fn l64(gas: u64) -> u64 {
    gas - gas / 64
}


struct CallInterrupt {}
struct CreateInterrupt {}

struct Executor<'config, State: StackState> {
    state: Box<State>,
    config: &'config evm::Config,
}

impl<'config, State: StackState> Handler for Executor<'config, State> {
    type CreateInterrupt = crate::executor::CreateInterrupt;
    type CreateFeedback = Infallible;
    type CallInterrupt = crate::executor::CallInterrupt;
    type CallFeedback = Infallible;

    fn balance(&self, address: H160) -> U256 {
        self.state.basic(address).balance
    }

    fn code_size(&self, address: H160) -> U256 {
        U256::from(self.state.code_size(address))
    }

    fn code_hash(&self, address: H160) -> H256 {
        if !self.exists(address) {
            return H256::default()
        }

        self.state.code_hash(address)
    }

    fn code(&self, address: H160) -> Vec<u8> {
        self.state.code(address)
    }

    fn storage(&self, address: H160, index: H256) -> H256 {
        self.state.storage(address, index)
    }

    fn original_storage(&self, address: H160, index: H256) -> H256 {
        self.state.original_storage(address, index).unwrap_or_default()
    }

    fn gas_left(&self) -> U256 {
        U256::one() // U256::from(self.state.metadata().gasometer.gas())
    }

    fn gas_price(&self) -> U256 {
        self.state.gas_price()
    }

    fn origin(&self) -> H160 {
        self.state.origin()
    }

    fn block_hash(&self, number: U256) -> H256 {
        self.state.block_hash(number)
    }

    fn block_number(&self) -> U256 {
        self.state.block_number()
    }

    fn block_coinbase(&self) -> H160 {
        self.state.block_coinbase()
    }

    fn block_timestamp(&self) -> U256 {
        self.state.block_timestamp()
    }

    fn block_difficulty(&self) -> U256 {
        self.state.block_difficulty()
    }

    fn block_gas_limit(&self) -> U256 {
        self.state.block_gas_limit()
    }

    fn chain_id(&self) -> U256 {
        self.state.chain_id()
    }

    fn exists(&self, address: H160) -> bool {
        if self.config.empty_considered_exists {
            self.state.exists(address)
        } else {
            self.state.exists(address) && !self.state.is_empty(address)
        }
    }

    fn deleted(&self, address: H160) -> bool {
        self.state.deleted(address)
    }

    fn set_storage(&mut self, address: H160, index: H256, value: H256) -> Result<(), ExitError> {
        self.state.set_storage(address, index, value);
        Ok(())
    }

    fn log(&mut self, address: H160, topics: Vec<H256>, data: Vec<u8>) -> Result<(), ExitError> {
        self.state.log(address, topics, data);
        Ok(())
    }

    fn mark_delete(&mut self, address: H160, target: H160) -> Result<(), ExitError> {
        let balance = self.balance(address);

        self.state.transfer(evm::Transfer {
            source: address,
            target: target,
            value: balance,
        })?;
        self.state.reset_balance(address);
        self.state.set_deleted(address);

        Ok(())
    }

    fn create(
        &mut self,
        caller: H160,
        scheme: evm::CreateScheme,
        value: U256,
        init_code: Vec<u8>,
        target_gas: Option<usize>,
    ) -> Capture<(ExitReason, Option<H160>, Vec<u8>), Self::CreateInterrupt> {
        Capture::Trap(CreateInterrupt{})
    }

    fn call(
        &mut self,
        code_address: H160,
        transfer: Option<evm::Transfer>,
        input: Vec<u8>,
        target_gas: Option<usize>,
        is_static: bool,
        context: evm::Context,
    ) -> Capture<(ExitReason, Vec<u8>), Self::CallInterrupt> {
        if let Some(depth) = self.state.metadata().depth() {
            if depth + 1 > self.config.call_stack_limit {
                return Capture::Exit((ExitError::CallTooDeep.into(), Vec::new()));
            }
        }

        Capture::Trap(CallInterrupt{})
    }

    fn pre_validate(
        &mut self,
        context: &evm::Context,
        opcode: Result<evm::Opcode, evm::ExternalOpcode>,
        stack: &evm::Stack,
    ) -> Result<(), ExitError> {
        // if let Some(cost) = gasometer::static_opcode_cost(opcode) {
        //     self.state.metadata_mut().gasometer.record_cost(cost)?;
        // } else {
        //     let is_static = self.state.metadata().is_static;
        //     let (gas_cost, memory_cost) = gasometer::dynamic_opcode_cost(
        //         context.address, opcode, stack, is_static, &self.config, self
        //     )?;

        //     let gasometer = &mut self.state.metadata_mut().gasometer;

        //     gasometer.record_dynamic_cost(gas_cost, memory_cost)?;
        // }
        Ok(())
    }
}


pub struct Machine<'config, State: StackState> {
    executor: Executor<'config, State>,
    runtime: Vec<evm::Runtime<'config>>
}


impl<'config, State: StackState> Machine<'config, State> {

    pub fn new(state: Box<State>, config: &'config evm::Config) -> Box<Self> {
        let executor = Executor { state, config };
        Box::new(Self{ executor, runtime: Vec::new() })
    }

    pub fn restore(pointer: *mut Self) -> Box<Self> {
        unsafe { Box::from_raw(pointer) }
    }

    pub fn call_begin(&mut self, caller: H160, code_address: H160, input: Vec<u8>, gas_limit: u64) {
        self.executor.state.inc_nonce(caller);


        // let after_gas = if take_l64 && self.config.call_l64_after_gas {
        //     if self.config.estimate {
        //         let initial_after_gas = self.state.metadata().gasometer.gas();
        //         let diff = initial_after_gas - l64(initial_after_gas);
        //         try_or_fail!(self.state.metadata_mut().gasometer.record_cost(diff));
        //         self.state.metadata().gasometer.gas()
        //     } else {
        //         l64(self.state.metadata().gasometer.gas())
        //     }
        // } else {
        //     self.state.metadata().gasometer.gas()
        // };

        // let mut gas_limit = min(gas_limit, after_gas);

        // try_or_fail!(
        //     self.state.metadata_mut().gasometer.record_cost(gas_limit)
        // );

        self.executor.state.enter(gas_limit, false);
        self.executor.state.touch(code_address);


        let code = self.executor.code(code_address);
        let context = evm::Context{address: code_address, caller: caller, apparent_value: U256::zero()};

        let runtime = evm::Runtime::new(Rc::new(code), Rc::new(input), context, &self.executor.config);
        self.runtime.push(runtime);
    }

    pub fn step(&mut self) -> Result<(), ExitReason> {

        if let Some(runtime) = self.runtime.last_mut() {
            match runtime.step(&mut self.executor) {
                Ok(()) => return Ok(()),
                Err(capture) => match capture {
                    Capture::Exit(reason) => {
                        match &reason {
                            ExitReason::Succeed(_) => {
                                self.executor.state.exit_commit().unwrap();
                                // todo call interrupt
                            },
                            ExitReason::Error(_) => self.executor.state.exit_discard().unwrap(),
                            ExitReason::Revert(_) => self.executor.state.exit_revert().unwrap(),
                            ExitReason::Fatal(_) => self.executor.state.exit_discard().unwrap()
                        }
                        return Err(reason);
                    },
                    Capture::Trap(resolve) => return Err(ExitReason::Fatal(ExitFatal::NotSupported)) // todo
                }
            }
        }

        Err(ExitReason::Fatal(ExitFatal::NotSupported))
    }

    #[must_use]
    pub fn return_value(&self) -> Vec<u8> {
        if let Some(runtime) = self.runtime.last() {
            return runtime.machine().return_value();
        }

        Vec::new()
    }

    pub fn into_state(self) -> Box<State> {
        self.executor.state
    }
}