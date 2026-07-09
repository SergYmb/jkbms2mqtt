mod aggregator;
mod freeze;
mod handle;

#[allow(clippy::module_inception)]
mod coordinator;

pub use handle::CoordinatorHandle;

#[cfg(test)]
mod tests;
