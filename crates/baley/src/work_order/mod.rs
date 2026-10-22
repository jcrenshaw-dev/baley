//! Work orders over the storage port (design 0003 section 6, design 0012):
//! the route an order is issued under, the recording of an issued order and
//! the reads of orders by id and of the pending start order, and the
//! recording of a failure. Project start, and the failure and retry of an
//! order, issue and read orders through here, as will the builds that issue
//! planner, checker and review orders.
//! The pure half, the records and the judges, is `baley_core::work_order`.

/// The route an order is issued under: the host's accepted model names from
/// the per-user catalog, and the route record and agent stub composed from them.
pub mod route;

/// The recording of one issued order: its inputs as `record` payloads and one
/// `work_order.issued` on its subject's stream.
pub mod issue;

/// The reads of an order by its id and of the pending start order.
pub mod read;

/// The recording of one order's failure: one `work_order.failed` on the
/// stream the order was issued on.
pub mod fail;
