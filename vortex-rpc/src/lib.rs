use rkyv::{Archive, Deserialize, Serialize};

/// VORTEX Binary Protocol (VBP) Header
/// Fixed 16-byte header for SIMD alignment.
#[derive(Archive, Deserialize, Serialize, Debug, Clone, Copy)]
#[archive(check_bytes)]
#[repr(C, align(16))]
pub struct VbpHeader {
    pub magic: u16,        // 0x5658 (VX)
    pub version: u8,
    pub command_code: u8,
    pub correlation_id: u32,
    pub payload_len: u32,
    pub flags: u32,
}

#[derive(Archive, Deserialize, Serialize, Debug, Clone, Copy, PartialEq, Eq)]
#[archive(check_bytes)]
#[repr(u8)]
pub enum Command {
    Ping = 0,
    Upsert = 1,
    Query = 2,
    Delete = 3,
    Stats = 4,
}

/// Example Payload for Vector Upsert
#[derive(Archive, Deserialize, Serialize, Debug, Clone)]
#[archive(check_bytes)]
#[repr(C, align(64))]
pub struct UpsertRequest {
    pub vector_id: u64,
    pub dimension: u32,
    pub embedding: Vec<f32>,
}

pub const VBP_MAGIC: u16 = 0x5658;
pub const VBP_VERSION: u8 = 1;
