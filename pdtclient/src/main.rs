use std::net::{SocketAddr, TcpStream};
use std::process::Command;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use pdtcore::*;

struct ClientConnection {
    addr: SocketAddr,
}

impl ClientConnection {
    fn connect(self) -> Result<(Client, TcpStream), ClientError> {
        let tcp_stream = TcpStream::connect(self.addr).map_err(ClientError::Connect)?;
        let read = tcp_stream.try_clone().map_err(ClientError::Connect)?;

        let client = Client::new(tcp_stream)?;

        Ok((client, read))
    }
}

#[derive(Debug)]
struct Client {
    tcp_stream: TcpStream,
    active_flag_ref: Particularity<bool>,
}

#[derive(Debug)]
enum ClientError {
    Connect(std::io::Error),
    Send(ProtocolError),
    Receive(ProtocolError),
    Shutdown(std::io::Error),
}

impl Client {
    fn new(tcp_stream: TcpStream) -> Result<Self, ClientError> {
        Ok(Self {
            active_flag_ref: Arc::new(Mutex::new(true)),
            tcp_stream,
        })
    }

    fn handle_message(&mut self, message: Message) {
        match message {
            Message::Server(_) => unreachable!(),
            Message::Common(message) => match message {
                CommonMessage::End => {
                    self.write_active_flag(false);
                }
            },
            Message::Client(action) => match action {
                ClientMessage::ScreenOff => {
                    Command::new("xset")
                        .args(["dpms", "force", "off"])
                        .spawn()
                        .unwrap()
                        .wait()
                        .unwrap();
                }
                ClientMessage::PowerOff => todo!(),
                ClientMessage::Restart => todo!(),
            },
        }
    }

    fn write_active_flag(&mut self, target: bool) {
        let mut guard = self.active_flag_ref.lock().unwrap();
        let active = &mut *guard;
        *active = target;
    }

    fn read_active_flag(&self) -> bool {
        let guard = self.active_flag_ref.lock().unwrap();
        *guard
    }

    fn end(&mut self) -> Result<(), ClientError> {
        Message::from(CommonMessage::End)
            .send(&mut self.tcp_stream)
            .map_err(ClientError::Send)?;

        self.tcp_stream
            .shutdown(std::net::Shutdown::Both)
            .map_err(ClientError::Shutdown)?;

        Ok(())
    }

    fn introduction(&mut self) -> Result<(), ClientError> {
        let device_info = pdtcore::DeviceInfo {
            name: String::from("ASH"),
        };

        Message::from(ServerMessage::Introduction(device_info))
            .send(&mut self.tcp_stream)
            .map_err(ClientError::Send)?;

        Ok(())
    }

    fn handle_error(&mut self, error: ProtocolError) -> Result<(), ClientError> {
        if !self.read_active_flag() {
            return self.end();
        }

        match self.end() {
            Ok(_) => Err(ClientError::Receive(error)),
            Err(error) => Err(error),
        }
    }

    fn run(&mut self) -> Result<(), ClientError> {
        self.introduction()?;

        while self.read_active_flag() {
            match Message::receive(&mut self.tcp_stream) {
                Ok(message) => self.handle_message(message),
                Err(error) => self.handle_error(error)?,
            }
        }

        Ok(())
    }
}

fn main() {
    let addr = SocketAddr::from_str("127.0.0.1:2039").unwrap();

    let (mut client, tcp_stream) = ClientConnection { addr }.connect().unwrap();

    let active_flag_ref = client.active_flag_ref.clone();

    std::thread::spawn(move || {
        client.run().unwrap();
    });

    let mut signals = signal_hook::iterator::Signals::new([
        signal_hook::consts::SIGTERM,
        signal_hook::consts::SIGINT,
    ])
    .unwrap();

    signals.forever().next().unwrap();

    *active_flag_ref.lock().unwrap() = false;

    tcp_stream.shutdown(std::net::Shutdown::Read).unwrap();

    std::thread::sleep(Duration::from_millis(100));
}
