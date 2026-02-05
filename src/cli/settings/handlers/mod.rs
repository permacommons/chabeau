//! Setting handlers for different configuration patterns.

pub mod boolean;
pub mod mcp;
pub mod provider_keyed;
pub mod provider_model_keyed;
pub mod simple;
pub mod string;

pub use boolean::*;
pub use mcp::*;
pub use provider_keyed::*;
pub use provider_model_keyed::*;
pub use simple::*;
pub use string::*;
