//! kyoto-core: everything of the editor that does not depend on a terminal.
//!
//! * [`buffer`]: line based text storage with grapheme-aware helpers
//! * [`editor`]: cursor, selection, undo/redo, clipboard, search, commands
//! * [`keys`]: key types, key string parsing and the default CUA key map
//! * [`config`]: configuration written in the Koto language

pub mod buffer;
pub mod color;
pub mod command;
pub mod config;
pub mod editor;
pub mod keys;

pub use buffer::{Buffer, Pos};
pub use color::{Color, Theme};
pub use command::{Command, Motion};
pub use config::{Config, ConfigError, Loaded, Script};
pub use editor::{Editor, Effect, PromptKind};
pub use keys::{Binding, Key, KeyCode, KeyMap, Modifiers};
