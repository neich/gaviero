pub mod types;
pub mod workspace;
pub mod session_state;
pub mod tree_sitter;
pub mod diff_engine;
pub mod observer;
pub mod write_gate;
pub mod acp;
pub mod git;
pub mod indent;
pub mod query_loader;
pub mod memory;
pub mod swarm;
pub mod terminal;

// Re-export tree-sitter types for downstream crates
pub use ::tree_sitter::Language;
pub use ::tree_sitter::Tree;
pub use ::tree_sitter::Parser;
pub use ::tree_sitter::Query;
pub use ::tree_sitter::QueryCursor;
pub use ::tree_sitter::InputEdit;
pub use ::tree_sitter::Node;
pub use ::tree_sitter::Point;
