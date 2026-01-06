use rkyv::{Archive, Deserialize, Serialize};
use bytecheck::CheckBytes;

/// 'VX' in ASCII hex. Used to identify VORTEX Binary Protocol packets.
pub const VBP_MAGIC: u16 = 0x5658;

/// Opcode for inserting or updating a vector.
pub const OP_UPSERT: u8 = 1;

/// Opcode for searching nearest neighbors.
pub const OP_SEARCH: u8 = 5;

/// The strict layout of the VORTEX Binary Protocol Header.
/// 
/// # Layout (C-Compatible)
/// - `magic` (2 bytes): Must be `0x5658`.
/// - `version` (1 byte): Protocol version (currently 1).
/// - `opcode` (1 byte): Command type (1=Upsert, 5=Search).
/// - `payload_len` (4 bytes): Length of the following payload body.
/// - `request_id` (8 bytes): Client-generated correlation ID.
/// 
/// # Alignment
/// This struct uses `#[repr(C)]`. The natural alignment of fields matches the packed layout perfectly
/// (2+1+1 = 4 bytes offset for u32, 4+4 = 8 bytes offset for u64).
/// This avoids "reference to packed field" errors in Rust while maintaining the exact binary layout.
#[repr(C)]
#[derive(Archive, Serialize, Deserialize, Debug, Clone, Copy)]
#[archive_attr(derive(CheckBytes, Debug))]
pub struct RequestHeader {
    pub magic: u16,
    pub version: u8,
    pub opcode: u8,
    pub payload_len: u32,
    pub request_id: u64,
}

/// Safely casts a byte slice to a RequestHeader and validates the magic number.
///
/// # Errors
/// Returns an error if the slice is too short or the magic number is invalid.
///
/// # Safety
/// This function handles the unsafe pointer cast internally and verifies bounds.
pub fn verify_header(bytes: &[u8]) -> Result<&RequestHeader, &'static str> {
    if bytes.len() < std::mem::size_of::<RequestHeader>() {
        return Err("Packet too short for VBP Header");
    }

    // SAFETY: We checked the length above. The struct is POD (Archive+Copy+C-Repr).
    // The pointer cast is valid for reading raw bytes as the struct.
    let header = unsafe { &*(bytes.as_ptr() as *const RequestHeader) };

    if header.magic != VBP_MAGIC {
        return Err("Invalid Magic Number");
    }

    Ok(header)
}
