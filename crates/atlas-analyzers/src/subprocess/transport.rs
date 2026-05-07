//! Length-prefixed JSON framing for the subprocess analyser
//! protocol.
//!
//! On-wire shape, per [`write_frame`] and [`read_frame`]:
//!
//! ```text
//! +--------+----------------------------+
//! | 4 byte |   N bytes of UTF-8 JSON    |
//! | u32 BE |                            |
//! +--------+----------------------------+
//! ```
//!
//! The leading `u32` is the length of the JSON payload in bytes,
//! big-endian. The payload is whatever JSON the caller asked to
//! write; this module does not interpret it.
//!
//! ## Why length-prefixed and not line-delimited
//!
//! Subprocess analysers may emit JSON with embedded newlines (a
//! pretty-printed payload, a string literal containing `\n`, ...).
//! Length-prefixing avoids the need to escape newlines in the
//! payload and keeps the framing trivial for the child to emit:
//! it serialises once, computes length, writes 4 bytes then the
//! body. The transport never has to scan a buffer for delimiters.
//!
//! ## Errors
//!
//! Frame I/O errors propagate as [`std::io::Error`]; the parent
//! translates them into [`crate::AnalyzerError::CallFailed`] when
//! the subprocess proxy catches them. EOF on the length prefix is
//! reported as `UnexpectedEof`; EOF mid-payload is reported as
//! `UnexpectedEof` too (a partial write is fatal).
//!
//! ## Frame size limit
//!
//! Frames larger than [`MAX_FRAME_BYTES`] (16 MiB) are rejected on
//! both write and read with `InvalidData`. This is a safety valve:
//! a runaway analyser could OOM the parent by streaming a
//! gigantic payload, so we cap the legal size and let the proxy
//! kill the child.

use std::io::{self, Read, Write};

/// Largest JSON payload the framing accepts. 16 MiB is well
/// beyond any realistic analyser response and small enough that
/// a single frame fits comfortably in memory on every supported
/// platform.
pub const MAX_FRAME_BYTES: u32 = 16 * 1024 * 1024;

/// Write a single length-prefixed JSON frame to `dst`.
///
/// Encodes `payload.len()` as a 4-byte big-endian `u32`, then the
/// payload bytes as-is. Flushes the writer afterwards so a buffered
/// child sees the request immediately. Returns an error if the
/// payload is larger than [`MAX_FRAME_BYTES`].
pub fn write_frame<W: Write>(dst: &mut W, payload: &[u8]) -> io::Result<()> {
    let len: u32 = payload
        .len()
        .try_into()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "frame larger than u32::MAX"))?;
    if len > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("frame {len} bytes exceeds {MAX_FRAME_BYTES} byte cap"),
        ));
    }
    dst.write_all(&len.to_be_bytes())?;
    dst.write_all(payload)?;
    dst.flush()?;
    Ok(())
}

/// Read a single length-prefixed JSON frame from `src`. Returns
/// the payload bytes (length already stripped). Errors:
///
/// - `UnexpectedEof` on EOF before the 4-byte length is fully
///   read, or on EOF before the payload is fully read.
/// - `InvalidData` when the announced length exceeds
///   [`MAX_FRAME_BYTES`].
/// - Other I/O errors propagate.
pub fn read_frame<R: Read>(src: &mut R) -> io::Result<Vec<u8>> {
    let mut len_buf = [0u8; 4];
    src.read_exact(&mut len_buf)?;
    let len = u32::from_be_bytes(len_buf);
    if len > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("frame {len} bytes exceeds {MAX_FRAME_BYTES} byte cap"),
        ));
    }
    let mut buf = vec![0u8; len as usize];
    src.read_exact(&mut buf)?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn transport_round_trip() {
        // Acceptance criterion: write a frame, read it back, assert
        // byte equality.
        let payload = br#"{"hello":"world"}"#;
        let mut buf: Vec<u8> = Vec::new();
        write_frame(&mut buf, payload).expect("write_frame succeeds");
        let mut cursor = Cursor::new(buf);
        let read_back = read_frame(&mut cursor).expect("read_frame succeeds");
        assert_eq!(read_back, payload);
    }

    #[test]
    fn transport_round_trip_empty_payload() {
        // A zero-length payload is valid (the `len` is 0).
        let mut buf: Vec<u8> = Vec::new();
        write_frame(&mut buf, b"").expect("write_frame on empty payload");
        // Frame is exactly the 4-byte length.
        assert_eq!(buf, vec![0u8, 0, 0, 0]);
        let mut cursor = Cursor::new(buf);
        let read_back = read_frame(&mut cursor).expect("read_frame on empty payload");
        assert!(read_back.is_empty());
    }

    #[test]
    fn transport_round_trip_multiple_frames() {
        // Two frames in a stream are independently decodable.
        let mut buf: Vec<u8> = Vec::new();
        write_frame(&mut buf, b"first").unwrap();
        write_frame(&mut buf, b"second").unwrap();
        let mut cursor = Cursor::new(buf);
        let f1 = read_frame(&mut cursor).unwrap();
        let f2 = read_frame(&mut cursor).unwrap();
        assert_eq!(f1, b"first");
        assert_eq!(f2, b"second");
    }

    #[test]
    fn read_frame_errors_on_truncated_length() {
        // Only 2 of 4 length bytes; read_frame must report
        // UnexpectedEof.
        let bytes = vec![0u8, 0];
        let mut cursor = Cursor::new(bytes);
        let err = read_frame(&mut cursor).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn read_frame_errors_on_truncated_payload() {
        // Length declares 5 bytes but only 2 follow.
        let mut bytes = vec![0u8, 0, 0, 5];
        bytes.extend_from_slice(b"ab");
        let mut cursor = Cursor::new(bytes);
        let err = read_frame(&mut cursor).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn read_frame_rejects_oversize_announcement() {
        // u32 max is well beyond MAX_FRAME_BYTES.
        let bytes = vec![0xff, 0xff, 0xff, 0xff];
        let mut cursor = Cursor::new(bytes);
        let err = read_frame(&mut cursor).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn write_frame_rejects_oversize_payload() {
        // Construct a payload that exceeds the cap. Don't actually
        // allocate 16 MiB+1 bytes; instead use a writer that
        // succeeds but exercises the size guard via a cheap
        // sentinel. The cheapest test is to pass a slice whose
        // declared len exceeds MAX_FRAME_BYTES — but `[u8]` can't
        // lie about its length. We build a real >MAX_FRAME_BYTES
        // payload because the constant is small enough at 16 MiB
        // that the test still completes in well under a second.
        let big = vec![0u8; (MAX_FRAME_BYTES as usize) + 1];
        let mut sink: Vec<u8> = Vec::new();
        let err = write_frame(&mut sink, &big).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }
}
