use std::io::Write;
use std::net::TcpStream;
use std::mem;
use vortex_rpc::{RequestHeader, VBP_MAGIC, OP_UPSERT};

fn main() -> std::io::Result<()> {
    let addr = "127.0.0.1:9000";
    println!("Connecting to VORTEX at {}...", addr);
    let mut stream = TcpStream::connect(addr)?;

    println!("Connected. Constructing VBP Upsert Packet...");

    // 4 bytes payload: 0xDE, 0xAD, 0xBE, 0xEF
    let payload = [0xDE, 0xAD, 0xBE, 0xEF];
    
    let header = RequestHeader {
        magic: VBP_MAGIC,
        version: 1,
        opcode: OP_UPSERT,
        payload_len: payload.len() as u32,
        request_id: 1,
    };

    // Serialize Header (Raw C-Layout)
    // SAFETY: RequestHeader is #[repr(C)] (Strict Layout) and contains only POD types. 
    // We cast it to a byte slice to send over the wire. This is the definition of "The Contract".
    let header_bytes = unsafe {
        std::slice::from_raw_parts(
            &header as *const RequestHeader as *const u8,
            mem::size_of::<RequestHeader>()
        )
    };

    println!("Sending Header ({} bytes) + Payload ({} bytes)...", header_bytes.len(), payload.len());
    
    stream.write_all(header_bytes)?;
    stream.write_all(&payload)?;
    stream.flush()?;

    println!("Probe dispatched successfully.");
    Ok(())
}
