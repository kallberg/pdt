use std::net::{SocketAddr, TcpStream};
use std::process::Command;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use pdtcore::*;
use tracing::{info, instrument, warn};
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::prelude::__tracing_subscriber_SubscriberExt;
use tracing_subscriber::{EnvFilter, Registry};

#[derive(Debug)]
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
    shutdown_request_flag_ref: Particularity<bool>,
}

#[derive(Debug)]
enum ClientError {
    Connect(std::io::Error),
    Send(ProtocolError),
    Receive(ProtocolError),
    Shutdown(std::io::Error),
    Command(std::io::Error),
    Closed,
}

impl Client {
    fn new(tcp_stream: TcpStream) -> Result<Self, ClientError> {
        Ok(Self {
            shutdown_request_flag_ref: Arc::new(Mutex::new(false)),
            tcp_stream,
        })
    }

    #[instrument(skip(self))]
    fn handle_message(&mut self, message: Message) -> Result<bool, ClientError> {
        info!(message =? message);

        match message {
            Message::Server(_) => unreachable!(),
            Message::Common(message) => match message {
                CommonMessage::End => {
                    self.request_shutdown();

                    info!("ending");
                    self.end()?;
                    return Ok(false);
                }
            },
            Message::Client(action) => match action {
                ClientMessage::ScreenOff => {
                    Command::new("xset")
                        .args(["dpms", "force", "off"])
                        .spawn()
                        .map_err(ClientError::Command)?
                        .wait()
                        .map_err(ClientError::Command)?;
                }
                ClientMessage::PowerOff => todo!(),
                ClientMessage::Restart => todo!(),
            },
        };

        Ok(true)
    }

    #[instrument(skip_all)]
    fn request_shutdown(&mut self) {
        let mut guard = self.shutdown_request_flag_ref.lock().unwrap();
        let shutdown_request_ref = &mut *guard;
        *shutdown_request_ref = false;
    }

    #[instrument(skip_all)]
    fn shutdown_requested(&self) -> bool {
        let guard = self.shutdown_request_flag_ref.lock().unwrap();
        *guard
    }

    #[instrument(skip_all)]
    fn end(&mut self) -> Result<(), ClientError> {
        Message::from(CommonMessage::End)
            .send(&mut self.tcp_stream)
            .map_err(ClientError::Send)?;

        self.tcp_stream
            .shutdown(std::net::Shutdown::Both)
            .map_err(ClientError::Shutdown)?;

        Ok(())
    }

    #[instrument(skip_all)]
    fn introduction(&mut self) -> Result<(), ClientError> {
        let device_info = pdtcore::DeviceInfo {
            name: String::from("ASH"),
        };

        Message::from(ServerMessage::Introduction(device_info))
            .send(&mut self.tcp_stream)
            .map_err(ClientError::Send)?;

        Ok(())
    }

    fn reconnect(&mut self) -> Result<(), ClientError> {
        if self.shutdown_requested() {
            return Err(ClientError::Closed);
        }
        let peer_addr = self.tcp_stream.peer_addr().map_err(ClientError::Connect)?;
        info!(addr =? peer_addr, "reconnecting");

        self.tcp_stream = TcpStream::connect(peer_addr).map_err(ClientError::Connect)?;

        self.introduction()?;

        Ok(())
    }

    #[instrument(skip_all)]
    fn receive(&mut self, max_retries: usize) -> Result<Option<Message>, ClientError> {
        if self.shutdown_requested() {
            warn!("shutdown requested not reading more messages");
            return Ok(None);
        }
        match Message::receive(&mut self.tcp_stream) {
            Ok(message) => {
                info!(message =? message);
                Ok(Some(message))
            }
            Err(error) => {
                if self.shutdown_requested() {
                    info!("shutdown requested");
                    Ok(None)
                } else {
                    let mut retry = 0;

                    while retry <= max_retries {
                        retry += 1;
                        warn!(retry = retry, max_retries = max_retries, "connection lost");

                        match self.reconnect() {
                            Ok(_) => {
                                info!(retry = retry, max_retries = max_retries, "reconnected");
                                return self.receive(max_retries);
                            }
                            Err(_) => {
                                if retry > 10 {
                                    std::thread::sleep(Duration::from_millis(1000 * retry as u64))
                                } else {
                                    std::thread::sleep(Duration::from_millis(1000));
                                }
                            }
                        }
                    }

                    Err(ClientError::Receive(error))
                }
            }
        }
    }

    #[instrument(skip_all)]
    fn run(&mut self) -> Result<(), ClientError> {
        self.introduction()?;

        while let Some(message) = self.receive(20)? {
            info!(message =? message);
            if !self.handle_message(message)? {
                info!("ending");
                return self.end();
            }
        }

        info!("no more messages to process");
        self.end()
    }
}

fn setup_tracing() {
    let layer = tracing_logfmt::builder().with_target(false).layer();

    let filter = match EnvFilter::try_from_default_env() {
        Ok(filter) => filter,
        _ => EnvFilter::builder()
            .with_default_directive(LevelFilter::INFO.into())
            .parse("")
            .unwrap(),
    };

    let subscriber = Registry::default().with(layer).with(filter);
    tracing::subscriber::set_global_default(subscriber).unwrap();
}

#[instrument]
fn main() {
    setup_tracing();

    let addr = SocketAddr::from_str("127.0.0.1:2039").unwrap();

    let (mut client, _) = ClientConnection { addr }.connect().unwrap();

    match client.run() {
        Ok(_) => info!("goodbye"),
        Err(error) => warn!(error =? error, "exited"),
    }
}
