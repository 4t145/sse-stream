#![doc = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/README.md"))]

// reference: https://html.spec.whatwg.org/multipage/server-sent-events.html

mod body;
mod error;
mod event;
mod stream;

pub use body::*;
pub use error::*;
pub use event::*;
pub use stream::*;
