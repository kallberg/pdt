use bincode::{
    error::{DecodeError, EncodeError},
    Decode, Encode,
};
use std::{
    io::{BufReader, Read, Write},
    sync::{Arc, Mutex},
};

mod built_info {
    include!(concat!(env!("OUT_DIR"), "/built.rs"));
}

pub type Particularity<T> = Arc<Mutex<T>>;

#[derive(Encode, Decode, Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    pub name: String,
    pub os: String,
    pub os_version: String,
    pub uptime: String,
}

impl Default for DeviceInfo {
    fn default() -> Self {
        Self {
            name: String::from("unknown"),
            os: String::from("unknown"),
            os_version: String::from("unknown"),
            uptime: String::from("unknown"),
        }
    }
}

#[derive(Debug)]
pub struct Client {
    pub id: String,
    pub device_info: DeviceInfo,
}

#[derive(Encode, Decode, Debug, Clone, PartialEq, Eq)]
pub struct BuiltInfo {
    pub pkg_version: String,
    pub pkg_version_major: String,
    pub pkg_version_minor: String,
    pub pkg_version_patch: String,
    pub pkg_version_pre: String,
    pub target: String,
    pub host: String,
    pub profile: String,
}

impl Default for BuiltInfo {
    fn default() -> Self {
        Self {
            pkg_version: built_info::PKG_VERSION.to_string(),
            pkg_version_major: built_info::PKG_VERSION_MAJOR.to_string(),
            pkg_version_minor: built_info::PKG_VERSION_MINOR.to_string(),
            pkg_version_patch: built_info::PKG_VERSION_PATCH.to_string(),
            pkg_version_pre: built_info::PKG_VERSION_PRE.to_string(),
            target: built_info::TARGET.to_string(),
            host: built_info::HOST.to_string(),
            profile: built_info::PROFILE.to_string(),
        }
    }
}

impl BuiltInfo {
    pub fn compatible(&self, other: &Self) -> bool {
        self.pkg_version_major == other.pkg_version_major
            && self.pkg_version_minor == other.pkg_version_minor
    }
}

#[derive(Encode, Decode, Debug, PartialEq, Eq)]
pub struct ClientIntroduction {
    pub name: String,
    pub pdtcore_built_info: BuiltInfo,
}

/// message for a client
#[derive(Encode, Decode, Debug, PartialEq, Eq)]
pub enum ClientMessage {
    ScreenOff,
    ScreenOn,
    PowerOff,
    Restart,
    Goodbye,
    RequestDeviceInfo,
}

/// message for a server
#[derive(Encode, Decode, Debug, PartialEq, Eq)]
pub enum ServerMessage {
    Hello(Box<ClientIntroduction>),
    DeviceInfo(DeviceInfo),
    Goodbye,
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

/// complete representation of pdt protocol messages
#[derive(Encode, Decode, Debug, PartialEq, Eq)]
pub enum Message {
    Client(ClientMessage),
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
