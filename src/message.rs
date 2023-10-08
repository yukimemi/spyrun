// =============================================================================
// File        : message.rs
// Author      : yukimemi
// Last Change : 2023/10/08 16:17:24.
// =============================================================================

#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    Event(notify::Event),
    Stop,
}
