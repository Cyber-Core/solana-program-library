use std::convert::Infallible;
use std::rc::Rc;
use evm_runtime::{save_return_value, save_created_address};
use evm::{ExternalOpcode};

use primitive_types::{H160, H256, U256};
use evm::{Capture, ExitError, ExitReason, ExitSucceed, ExitFatal, Handler, backend::Backend, Resolve};
use crate::executor_state::{ StackState, ExecutorState, ExecutorMetadata };
use std::mem;
use sha3::{Keccak256, Digest};
use std::borrow::BorrowMut;

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

fn keccak256_digest(data: &[u8]) -> H256 {
    H256::from_slice(Keccak256::digest(&data).as_slice())
}

struct CallInterrupt {
    code_address : H160,
    input : Vec<u8>,
    context: evm::Context,
}

struct CreateInterrupt {
    init_code: Vec<u8>,
    context: evm::Context,
    address: H160
}

struct Executor<'config, B: Backend> {
    state: ExecutorState<B>,
    config: &'config evm::Config,
}

impl<'config, B: Backend> Handler for Executor<'config, B> {
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
        init_code_: &Vec<u8>,
        target_gas: Option<usize>,
    ) -> Capture<(ExitReason, Option<H160>, Vec<u8>), Self::CreateInterrupt> {

        if let Some(depth) = self.state.metadata().depth() {
            if depth + 1 > self.config.call_stack_limit {
                return Capture::Exit((ExitError::CallTooDeep.into(), None, Vec::new()));
            }
        }
        // TODO: check
        // if self.balance(caller) < value {
        //     return Capture::Exit((ExitError::OutOfFund.into(), None, Vec::new()))
        // }

        // Get the create address from given scheme.
        let address =
            match scheme {
                evm::CreateScheme::Create2 { caller, code_hash, salt } => {
                    let mut hasher = Keccak256::new();
                    hasher.input(&[0xff]);
                    hasher.input(&caller[..]);
                    hasher.input(&salt[..]);
                    hasher.input(&code_hash[..]);
                    H256::from_slice(hasher.result().as_slice()).into()
                },
                evm::CreateScheme::Legacy { caller } => {
                    let nonce = self.state.basic(caller).nonce;
                    let mut stream = rlp::RlpStream::new_list(2);
                    stream.append(&caller);
                    stream.append(&nonce);
                    //H256::from_slice(Keccak256::digest(&stream.out()).as_slice()).into()
                    keccak256_digest(&stream.out()).into()
                },
                evm::CreateScheme::Fixed(naddress) => {
                    naddress
                },
            };

        // self.state.create(&scheme, &address);
        // TODO: may be increment caller's nonce after runtime creation or success execution?
        self.state.inc_nonce(caller);

        // if let code= self.state.code(address) {
        //     if code.len() != 0 {
        //         // let _ = self.merge_fail(substate);
        //         return Capture::Exit((ExitError::CreateCollision.into(), None, Vec::new()))
        //     }
        // }

        // if self.state.basic(address).nonce  > U256::zero() {
        //     return Capture::Exit((ExitError::CreateCollision.into(), None, Vec::new()))
        // }

        let context = evm::Context {
            address,
            caller,
            apparent_value: value,
        };

        let init_code:Vec<u8> = init_code_.clone();
        Capture::Trap(CreateInterrupt{init_code, context, address})
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

        let hook_res = self.state.call_inner(code_address, transfer, input.clone(), target_gas, is_static, true, true);
        if hook_res.is_some() {
            match hook_res.as_ref().unwrap() {
                Capture::Exit((reason, _return_data)) => {
                    return Capture::Exit((reason.clone(), _return_data.clone()))
                },
                Capture::Trap(_interrupt) => {
                    unreachable!("not implemented");
                },
            }
        }

        Capture::Trap(CallInterrupt{code_address, input, context})
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

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy)]
pub enum RuntimeReason {
    Call,
    Create(H160)
}

pub struct Machine<'config, B: Backend> {
    executor: Executor<'config, B>,
    runtime: Vec<(evm::Runtime<'config>, Option<RuntimeReason>)>
}


impl<'config, B: Backend> Machine<'config, B> {

    pub fn new(state: ExecutorState<B>) -> Self {
        let executor = Executor { state, config: evm::Config::default() };
        Self{ executor, runtime: Vec::new() }
    }

    pub fn save_into(&self, storage: &mut [u8]) {
        let machine_data = bincode::serialize(&self.runtime).unwrap();
        let executor_state_data = self.executor.state.save();
        
        bincode::serialize_into(storage, &(machine_data, executor_state_data)).unwrap();
    }

    pub fn restore(storage: &[u8], backend: B) -> Self {
        let (machine_data, state_data): (Vec<u8>, Vec<u8>) = bincode::deserialize(&storage).unwrap();
        let state = ExecutorState::restore(&state_data, backend);

        let executor = Executor { state, config: evm::Config::default() };
        Self{ executor, runtime: bincode::deserialize(&machine_data).unwrap() }
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
        self.runtime.push((runtime, None));
    }

    pub fn step(&mut self) -> Result<(), ExitReason> {

        enum modify<'a>{
            none,
            add((evm::Runtime<'a>, Option<RuntimeReason>)),
            remove(ExitReason),
        }
        let mut runtime_modify = modify::none;
        let runtime_cnt = self.runtime.len();
        if let Some(runtime) = self.runtime.last_mut() {
            match runtime.0.step(&mut self.executor) {
                Ok(()) => {},
                Err(capture) => match capture {
                    Capture::Exit(reason) => {
                        match &reason {
                            ExitReason::Succeed(res) => {
                                self.executor.state.exit_commit().unwrap();
                                if (runtime_cnt == 1){
                                    return Err(reason.clone());
                                } else{
                                    runtime_modify = modify::remove(reason.clone());
                                }
                            },
                            ExitReason::Error(_) => {
                                debug_print!("runtime.step: Err, capture Capture::Exit(reason), reason:ExitReason::Error(_)");
                                self.executor.state.exit_discard().unwrap();
                                return Err(reason.clone());
                            },
                            ExitReason::Revert(_) => {
                                debug_print!("runtime.step: Err, capture Capture::Exit(reason), reason:ExitReason::Revert(_)");
                                self.executor.state.exit_revert().unwrap();
                                return Err(reason.clone());
                            },
                            ExitReason::Fatal(_) => {
                                debug_print!("runtime.step: Err, capture Capture::Exit(reason), reason:ExitReason::Fatal(_)");
                                self.executor.state.exit_discard().unwrap();
                                return Err(reason.clone());
                            }
                        }
                    },
                    Capture::Trap(interrupt) => match interrupt{
                        Resolve::Call(interrupt, resolve) =>{
                            mem::forget(resolve);
                            let code = self.executor.code(interrupt.code_address);
                            self.executor.state.enter(u64::max_value(), false);
                            self.executor.state.touch(interrupt.code_address);

                            let mut runtime = evm::Runtime::new(
                                Rc::new(code),
                                Rc::new(interrupt.input),
                                interrupt.context,
                                &self.executor.config
                            );
                            runtime_modify = modify::add((runtime, Some(RuntimeReason::Call)));
                        },
                        Resolve::Create(interrupt, resolve) =>{
                            mem::forget(resolve);
                            self.executor.state.enter(u64::max_value(), false);
                            // self.executor.state.touch(interrupt.address);
                            // if self.executor.config.create_increase_nonce {
                            //     self.executor.state.inc_nonce(interrupt.address);
                            // }

                            let mut runtime = evm::Runtime::new(
                                Rc::new(interrupt.init_code),
                                Rc::new(Vec::new()),
                                interrupt.context,
                                &self.executor.config
                            );
                            runtime_modify = modify::add((runtime, Some(RuntimeReason::Create(interrupt.address))));
                        },
                        _ => {
                            debug_print!("runtime.step: Err, capture Capture::Trap(interrupt), interrupt: _");
                            return Err(ExitReason::Fatal(ExitFatal::NotSupported));
                        }
                    }
                }
            }
        }

        match runtime_modify {
            modify::remove(exit_reason) => {
                let mut return_value = Vec::new();
                let mut runtime_reason: Option<RuntimeReason> = None;
                if let Some(runtime) = self.runtime.last(){
                    return_value = runtime.0.machine().return_value();
                    runtime_reason = runtime.1;
                };
                self.runtime.pop();

                if let Some(runtime) = self.runtime.last_mut(){
                    match runtime_reason {
                        Some(RuntimeReason::Call) => {
                            let val =  save_return_value(
                                runtime.0.borrow_mut(),
                                exit_reason,
                                return_value,
                                &self.executor
                            );
                            // TODO check val
                        },
                        Some(RuntimeReason::Create(created_address)) => {
                            if let Some(limit) = self.executor.config.create_contract_limit {
                                if return_value.len() > limit {
                                    debug_print!("runtime.step: Err((ExitError::CreateContractLimit.into()))");
                                    self.executor.state.exit_discard().unwrap();
                                    return Err((ExitError::CreateContractLimit.into()))
                                    // TODO: may be continue ?
                                }
                            }
                            self.executor.state.set_code(created_address, return_value.clone());
                            let val =  save_created_address(
                                runtime.0.borrow_mut(),
                                exit_reason,
                                Some(created_address),
                                return_value,
                                &self.executor );
                            // TODO check val
                        },
                        None => {}
                    }
                }
            },
            modify::add(vm) => {
                self.runtime.push(vm);
            },
            modify::none => {},
        }
        return Ok(());
    }


    pub fn execute(&mut self) -> ExitReason {
        loop {
            if let Err(reason) = self.step() {
                return reason;
            }
        }
    }

    pub fn execute_n_steps(&mut self, n: u64) -> Result<(), ExitReason> {
        for i in 0..n {
            self.step()?;
        }

        Ok(())
    }

    #[must_use]
    pub fn return_value(&self) -> Vec<u8> {
        if let Some(runtime) = self.runtime.last() {
            return runtime.0.machine().return_value();
        }

        Vec::new()
    }

    pub fn into_state(self) -> ExecutorState<B> {
        self.executor.state
    }
}