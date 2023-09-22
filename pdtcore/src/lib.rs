pub mod pdt;

use bincode::{
    error::{DecodeError, EncodeError},
    Decode, Encode,
};
use std::{
    io::{BufReader, Read, Write},
    sync::{Arc, Mutex},
};

pub type Particularity<T> = Arc<Mutex<T>>;

#[derive(Encode, Decode, Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    pub name: String,
}

/// message for a client
#[derive(Encode, Decode, Debug, PartialEq, Eq)]
pub enum ClientMessage {
    ScreenOff,
    PowerOff,
    Restart,
}

/// message common to both client and server
#[derive(Encode, Decode, Debug, PartialEq, Eq)]
pub enum CommonMessage {
    End,
}

/// message for a server
#[derive(Encode, Decode, Debug, PartialEq, Eq)]
pub enum ServerMessage {
    Introduction(DeviceInfo),
}

impl From<ClientMessage> for Message {
    fn from(value: ClientMessage) -> Self {
        Self::Client(value)
    }
}

impl From<ServerMessage> for Message {
    fn from(value: ServerMessage) -> Self {
        Self::Server(value)
    }
}

impl From<CommonMessage> for Message {
    fn from(value: CommonMessage) -> Self {
        Self::Common(value)
    }
}

/// complete representation of pdt protocol messages
#[derive(Encode, Decode, Debug, PartialEq, Eq)]
pub enum Message {
    Client(ClientMessage),
    Common(CommonMessage),
    Server(ServerMessage),
}

#[derive(Debug)]
pub enum ProtocolError {
    IO(std::io::Error),
    Encode(EncodeError),
    Decode(DecodeError),
}

impl From<EncodeError> for ProtocolError {
    fn from(value: EncodeError) -> Self {
        ProtocolError::Encode(value)
    }
}

impl From<std::io::Error> for ProtocolError {
    fn from(value: std::io::Error) -> Self {
        ProtocolError::IO(value)
    }
}

impl From<DecodeError> for ProtocolError {
    fn from(value: DecodeError) -> Self {
        ProtocolError::Decode(value)
    }
}

/// read and write trait for pdt protocol
pub trait Protocol {
    fn send(&self, write_stream: &mut dyn Write) -> Result<(), ProtocolError>;
    fn receive(read_stream: &mut dyn Read) -> Result<Self, ProtocolError>
    where
        Self: std::marker::Sized;
}

/// read and write impl for pdt protocol
impl Protocol for Message {
    fn send(&self, write_stream: &mut dyn Write) -> Result<(), ProtocolError> {
        let bytes = bincode::encode_to_vec(self, bincode::config::standard())?;

        write_stream.write_all(&bytes)?;

        Ok(())
    }

    fn receive(read_stream: &mut dyn Read) -> Result<Self, ProtocolError> {
        let buf_read = BufReader::new(read_stream);

        let decoded: Message = bincode::decode_from_reader(buf_read, bincode::config::standard())?;

        Ok(decoded)
    }
}
