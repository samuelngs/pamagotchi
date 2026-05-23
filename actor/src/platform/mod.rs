mod adapter;
mod content;
mod router;
pub mod whatsapp;

pub use adapter::PlatformAdapter;
pub use content::{MediaAttachment, MediaKind, PlatformCapabilities};
pub use router::PlatformRouter;
