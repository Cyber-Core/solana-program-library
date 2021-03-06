//! A simple program that receives transfer operations from metamask-wrapper and transfers different tokens using another programs.

pub mod entrypoint;
pub mod error;
pub mod instruction;
pub mod processor;
pub mod state;

// Export current solana-sdk types for downstream users who may also be building with a different
// solana-sdk version
pub use solana_program;

solana_program::declare_id!("MetamaskW1111111111111111111111111111111111");
