//! A story's refinement over the storage port (design 0005 sections 5 to
//! 7, PLN-R1 to PLN-R6): the analyzer's orders, a story's question set,
//! truth submissions, their approval and the refinement read. Each is a
//! library function the session's operations and the owner's commands
//! call, so the terminal and the host reach the same rules (ADR 0044).

pub mod inputs;
pub mod order;
