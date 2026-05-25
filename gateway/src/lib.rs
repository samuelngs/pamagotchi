mod adapter;
mod content;
pub mod relay;
mod router;
pub mod whatsapp;

pub use adapter::GatewayAdapter;
pub use content::GatewayCapabilities;
pub use router::GatewayRouter;
