use std::io::{Read, Write};

use crate::errors::NativeError;
use crate::limits::{MAX_FRAME, WIRE_VERSION};
use crate::protocol::messages::Message;

pub(crate) fn read_frame<R: Read>(reader: &mut R) -> Result<Message, NativeError> {
    let mut len = [0u8; 4];
    reader.read_exact(&mut len[..1])?;
    reader
        .read_exact(&mut len[1..])
        .map_err(|_| NativeError::InvalidFrame)?;
    let len = u32::from_be_bytes(len) as usize;
    if len == 0 || len > MAX_FRAME {
        return Err(NativeError::InvalidFrame);
    }
    let mut data = vec![0; len];
    reader
        .read_exact(&mut data)
        .map_err(|_| NativeError::InvalidFrame)?;
    let msg: Message = serde_json::from_slice(&data)?;
    if msg.version() != WIRE_VERSION {
        return Err(NativeError::InvalidFrame);
    }
    Ok(msg)
}
/// Authenticated control sessions retain bounded partial frames across socket
/// polling timeouts. Pre-authentication still uses the strict one-shot reader.
#[derive(Default)]
pub(crate) struct FrameReader {
    header: [u8; 4],
    header_read: usize,
    data: Vec<u8>,
    data_read: usize,
}
impl FrameReader {
    pub(crate) fn read<R: Read>(&mut self, reader: &mut R) -> Result<Message, NativeError> {
        while self.header_read < self.header.len() {
            match reader.read(&mut self.header[self.header_read..]) {
                Ok(0) => return Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof).into()),
                Ok(count) => self.header_read += count,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error.into()),
            }
        }
        if self.data.is_empty() {
            let length = u32::from_be_bytes(self.header) as usize;
            if length == 0 || length > MAX_FRAME {
                return Err(NativeError::InvalidFrame);
            }
            self.data.resize(length, 0);
        }
        while self.data_read < self.data.len() {
            match reader.read(&mut self.data[self.data_read..]) {
                Ok(0) => return Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof).into()),
                Ok(count) => self.data_read += count,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error.into()),
            }
        }
        let message: Message = serde_json::from_slice(&self.data)?;
        if message.version() != WIRE_VERSION {
            return Err(NativeError::InvalidFrame);
        }
        self.header_read = 0;
        self.data_read = 0;
        self.data.clear();
        Ok(message)
    }
}

pub(crate) fn write_frame<W: Write>(writer: &mut W, message: &Message) -> Result<(), NativeError> {
    let data = serde_json::to_vec(message)?;
    if data.is_empty() || data.len() > MAX_FRAME {
        return Err(NativeError::InvalidFrame);
    }
    writer.write_all(&(data.len() as u32).to_be_bytes())?;
    writer.write_all(&data)?;
    writer.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn authenticated_reader_still_rejects_invalid_lengths_and_versions() {
        for size in [0, MAX_FRAME as u32 + 1, u32::MAX] {
            assert!(matches!(
                FrameReader::default().read(&mut size.to_be_bytes().as_slice()),
                Err(NativeError::InvalidFrame)
            ));
        }
        let payload = br#"{"type":"ping","protocol":999}"#;
        let mut wire = (payload.len() as u32).to_be_bytes().to_vec();
        wire.extend(payload);
        assert!(matches!(
            FrameReader::default().read(&mut wire.as_slice()),
            Err(NativeError::InvalidFrame)
        ));
    }
}
