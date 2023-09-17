pub mod pdt;

use bincode::{
    error::{DecodeError, EncodeError},
    Decode, Encode,
};
use std::io::{BufReader, Read, Write};

#[derive(Encode, Decode, Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    pub name: String,
}

#[derive(Encode, Decode, Debug, PartialEq, Eq)]
pub enum Message {
    DeviceInfo(DeviceInfo),
    ScreenOff,
    End,
}

#[derive(Debug)]
pub enum MessageProtocolError {
    IO(std::io::Error),
    Encode(EncodeError),
    Decode(DecodeError),
}

impl From<EncodeError> for MessageProtocolError {
    fn from(value: EncodeError) -> Self {
        MessageProtocolError::Encode(value)
    }
}

impl From<std::io::Error> for MessageProtocolError {
    fn from(value: std::io::Error) -> Self {
        MessageProtocolError::IO(value)
    }
}

impl From<DecodeError> for MessageProtocolError {
    fn from(value: DecodeError) -> Self {
        MessageProtocolError::Decode(value)
    }
}

pub trait MessageProtocol {
    fn send(&self, write_stream: &mut dyn Write) -> Result<(), MessageProtocolError>;
    fn receive(read_stream: &mut dyn Read) -> Result<Self, MessageProtocolError>
    where
        Self: std::marker::Sized;
}

impl MessageProtocol for Message {
    fn send(&self, write_stream: &mut dyn Write) -> Result<(), MessageProtocolError> {
        let bytes = bincode::encode_to_vec(self, bincode::config::standard())?;

        write_stream.write_all(&bytes)?;

        Ok(())
    }

    fn receive(read_stream: &mut dyn Read) -> Result<Self, MessageProtocolError> {
        let buf_read = BufReader::new(read_stream);

        let decoded: Message = bincode::decode_from_reader(buf_read, bincode::config::standard())?;

        Ok(decoded)
    }
}
