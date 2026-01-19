use std::io::{Write, Read};
use std::net::TcpStream;
use std::mem;
use vortex_rpc::{RequestHeader, VBP_MAGIC, OP_UPSERT, OP_SEARCH};

/// Pulse Check: Resurrection Edition
fn main() -> std::io::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let search_only = args.contains(&"--search-only".to_string());

    let addr = "127.0.0.1:8080";
    println!("Connecting to VORTEX at {}...", addr);
    let mut stream = TcpStream::connect(addr)?;

    if !search_only {
        println!("Connected. Constructing VBP Upsert Packet (128-dim Vector)...");

        // Payload: [ID (8 bytes)] + [Vector (128 * 4 = 512 bytes)]
        let id: u64 = 101;
        let vector: Vec<f32> = vec![0.1; 128]; 
        
        // PADDING LOGIC: VORTEX Constitution Rule 9 requires 4KB alignment.
        // Header (16) + ID (8) + Vector (128*4 = 512) = 536 bytes.
        // We set payload_len to the LOGICAL size (ID + Vector).
        let logical_payload_len = 8 + 512;
        let mut payload = Vec::with_capacity(logical_payload_len);
        
        payload.extend_from_slice(&id.to_le_bytes());
        for val in &vector {
            payload.extend_from_slice(&val.to_le_bytes());
        }
        
        let header = RequestHeader {
            magic: VBP_MAGIC,
            version: 1,
            opcode: OP_UPSERT,
            payload_len: logical_payload_len as u32,
            request_id: 1,
        };

        let header_bytes = unsafe {
            std::slice::from_raw_parts(
                &header as *const RequestHeader as *const u8,
                mem::size_of::<RequestHeader>()
            )
        };

        // We can still send 4096 bytes to ensure the server gets a full page if it wants,
        // but the header will tell it how much to actually process.
        let mut packet = Vec::with_capacity(4096);
        packet.extend_from_slice(header_bytes);
        packet.extend_from_slice(&payload);
        let padding_needed = 4096 - packet.len();
        packet.extend(std::iter::repeat(0u8).take(padding_needed));

        println!("Sending VBP Packet ({} bytes)...", packet.len());
        stream.write_all(&packet)?;
        stream.flush()?;

        println!("Waiting for ACK...");
        let mut response = [0u8; 16]; 
        stream.read_exact(&mut response)?;
        println!("Response Received: {:?}", response);
    } else {
        println!("Search-Only Mode Active (Skipping UPSERT)...");
    }

    println!("Sending SEARCH Packet...");
    let header_search = RequestHeader {
        magic: VBP_MAGIC,
        version: 1,
        opcode: OP_SEARCH,
        payload_len: 0, 
        request_id: 2,
    };
    let header_bytes_search = unsafe {
        std::slice::from_raw_parts(
            &header_search as *const RequestHeader as *const u8,
            mem::size_of::<RequestHeader>()
        )
    };
    stream.write_all(header_bytes_search)?;
    stream.flush()?;
    
    let mut response_search = [0u8; 16];
    stream.read_exact(&mut response_search)?;
    println!("Search Response Received: {:?}", response_search);

    println!("Probe Sequence Complete.");
    Ok(())
}
