//! Work orders' pure half (design 0003 section 6, design 0012): the route
//! record an order carries.
//!
//! Nothing here reads a store, a file or a clock: the binary supplies every
//! value it judges.

mod route;

pub use route::route_record;
