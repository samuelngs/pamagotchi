mod builder;
mod router;
mod types;

pub use builder::InferenceRouterBuilder;
pub use router::InferenceRouter;
pub use types::{Capability, InferenceEndpoint, Reasoning, ResolvedInference, RouteContext};

#[cfg(test)]
mod tests;
