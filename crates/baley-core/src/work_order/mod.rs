//! Work orders' pure half (design 0003 section 6, design 0012): the route
//! record an order carries and the two work-order events.
//!
//! Nothing here reads a store, a file or a clock: the binary supplies every
//! value it judges.

mod event;
mod route;

pub use event::{
    IssuedOrder, OrderInput, START_SUBJECT, WORK_ORDER_FAILED, WORK_ORDER_FAILED_VERSION,
    WORK_ORDER_ISSUED, WORK_ORDER_ISSUED_VERSION, agent_name, failed_payload, issued_payload,
    order_stream, register_work_order_events,
};
pub use route::route_record;
