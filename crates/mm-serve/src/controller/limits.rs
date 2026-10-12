//! Caps for one controller turn. Tests read these; they are not tuned per call.

/// Model rounds that may still propose tools. The next round summarizes and stops.
pub const MAX_MODEL_ROUNDS: u32 = 4;
/// Tool results in one turn. The next proposal is `tool step cap` and does not run.
pub const MAX_TOOL_STEPS: u32 = 6;
/// How many extra groups `catalog.open` may add beyond the ones already attached.
pub const MAX_EXTRA_GROUPS: u32 = 3;
/// Bytes of tool `data` copied into the next model message.
pub const MODEL_DATA_BYTES: usize = 16 * 1024;
