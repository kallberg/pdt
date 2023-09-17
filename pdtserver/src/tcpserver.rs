use std::{
    collections::HashMap,
    io::Read,
    io::Write,
    net::TcpListener,
    sync::{Arc, Mutex},
};

use pdtcore::{Message, MessageProtocol};
use ulid::Ulid;

use pdtcore::pdt::ClientInfo;

pub struct PDTSession {
    pub id: Ulid,
    pub client_info: Arc<Mutex<ClientInfo>>,
    pub receive: Arc<Mutex<dyn Read + Send>>,
    pub send: Arc<Mutex<dyn Write + Send>>,
}

/// PDT Server

pub struct PDTServer {
    pub graceful_exit: bool,
    pub tcp_listener: TcpListener,
    pub sessions: HashMap<Ulid, Arc<PDTSession>>,
}

pub enum PDTServerError {
    SendMutexLockError,
    SendNoSuchClient,
    TcpBindFailed,
    TcpStreamCloneFailed,
}

impl PDTServer {
    pub fn new(tcp_listener: TcpListener) -> Self {
        Self {
            graceful_exit: false,
            tcp_listener,
            sessions: HashMap::new(),
        }
    }

    /// Message to a client using channels
    pub fn send(&mut self, client_id: &Ulid, message: Message) -> Result<(), PDTServerError> {
        if let Some(session) = self.sessions.get(client_id).cloned() {
            if let Ok(mut guard) = session.send.lock() {
                message.send(&mut *guard).unwrap();
                return Ok(());
            }
            return Err(PDTServerError::SendMutexLockError);
        }

        Err(PDTServerError::SendNoSuchClient)
    }

    /// Turn screen off by engaging power save mode using xset
    pub fn screen_off(&mut self, client_id: &Ulid) -> Result<(), PDTServerError> {
        self.send(client_id, Message::ScreenOff)
    }

    /// Unregister and disconnect client
    pub fn end(&mut self, client_id: &Ulid) -> Result<(), PDTServerError> {
        self.send(client_id, Message::End)
    }

    /// Spawns two threads
    /// 1. Receiving clients forever until exit
    ///     - spawns worker threads
    /// 2. Incoming message handler thread
    ///     - reads messages from channel and updates server state
    pub fn serve(&mut self) -> Result<(), PDTServerError> {
        while !self.graceful_exit {
            let Ok((tcp_stream, address)) = self.tcp_listener.accept() else {
                return Err(PDTServerError::TcpBindFailed);
            };

            let id = Ulid::new();
            let client_info = ClientInfo::new("Unknown".to_string(), address.ip());

            let Ok(receive) = tcp_stream.try_clone() else {
                return Err(PDTServerError::TcpStreamCloneFailed);
            };

            let Ok(send) = tcp_stream.try_clone() else {
                return Err(PDTServerError::TcpStreamCloneFailed);
            };

            let receive = Arc::new(Mutex::new(receive));
            let send = Arc::new(Mutex::new(send));

            let session = Arc::new(PDTSession {
                id,
                client_info: Arc::new(Mutex::new(client_info)),
                receive,
                send,
            });

            self.sessions.insert(id, session.clone());

            std::thread::spawn(move || {
                let mut quit = false;

                while !quit {
                    let Ok(mut guard) = session.receive.lock() else {
                        // TODO figure out recovery and logs
                        return;
                    };

                    let stream = &mut *guard;

                    let Ok(message) = Message::receive(stream) else {
                        // TODO figure out recovery and logs
                        return;
                    };

                    match message {
                        Message::DeviceInfo(info) => {
                            let Ok(mut guard) = session.client_info.lock() else {
                                // TODO figure out recovery and logs
                                return;
                            };

                            let client_info = &mut *guard;

                            client_info.name = info.name
                        }
                        Message::ScreenOff => {}
                        Message::End => {
                            quit = true;
                            let Ok(_shutdown) = tcp_stream.shutdown(std::net::Shutdown::Read)
                            else {
                                // TODO figure out recovery and logs
                                return;
                            };
                        }
                    }
                }
            });
        }

        Ok(())
    }
}
