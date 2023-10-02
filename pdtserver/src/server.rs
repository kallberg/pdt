use std::{
    collections::HashMap,
    io::{Read, Write},
    net::TcpListener,
    sync::{
        mpsc::{self, RecvError},
        Arc, Mutex, MutexGuard, PoisonError,
    },
};

use pdtcore::{
    BuiltInfo, Client, ClientMessage, DeviceInfo, Message, ProtocolError, ServerMessage,
};
use pdtcore::{Particularity, Protocol};
use tracing::*;

use ulid::Ulid;

type AddressedMessage = (Ulid, Message);
type ClientSender = mpsc::Sender<Message>;
type ClientReceiver = mpsc::Receiver<Message>;
type ServerSender = mpsc::Sender<ServerEvent>;
type ServerReceiver = mpsc::Receiver<ServerEvent>;
type ServerSenderReference = Particularity<ServerSender>;
type ServerReceiverReference = Particularity<ServerReceiver>;
type ServerClientMap = Particularity<HashMap<Ulid, ServerClient>>;

#[derive(Debug)]
pub enum SendError {
    ClientNotFound,
    SendChannel,
    Deadlock,
}

#[derive(Debug)]
pub enum HandleError {
    Deadlock,
    ReceiveChannel,
}

#[derive(Debug)]
pub enum ReceiveError {
    Deadlock,
    ChannelSend,
}

impl<T> From<PoisonError<T>> for HandleError {
    fn from(_: PoisonError<T>) -> Self {
        HandleError::Deadlock
    }
}

impl From<RecvError> for HandleError {
    fn from(_value: RecvError) -> Self {
        HandleError::ReceiveChannel
    }
}

impl<T> From<PoisonError<T>> for ReceiveError {
    fn from(_: PoisonError<T>) -> Self {
        ReceiveError::Deadlock
    }
}

impl<T> From<mpsc::SendError<T>> for ReceiveError {
    fn from(_value: mpsc::SendError<T>) -> Self {
        ReceiveError::ChannelSend
    }
}

#[derive(Debug)]
pub enum ServerEvent {
    IncomingMessage(AddressedMessage),
    Unexpected(ProtocolError),
}

#[derive(Debug, Clone)]
pub struct ServerClient {
    pub id: Ulid,
    pub pdtcore_built_info: Option<BuiltInfo>,
    device_info: Option<DeviceInfo>,
    sender: ClientSender,
}

#[derive(Clone)]

pub struct Server {
    incoming_server_event_sender: ServerSenderReference,
    incoming_server_event_receiver: ServerReceiverReference,
    clients: ServerClientMap,
    client_ids: Particularity<Vec<Ulid>>,
}

impl Default for Server {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();

        Self {
            incoming_server_event_sender: Arc::new(Mutex::new(tx)),
            incoming_server_event_receiver: Arc::new(Mutex::new(rx)),
            clients: Arc::new(Mutex::new(HashMap::new())),
            client_ids: Arc::new(Mutex::new(vec![])),
        }
    }
}

impl Server {
    #[instrument(skip_all)]
    pub fn handle_messages(&self) -> Result<(), HandleError> {
        loop {
            let receiver = self.incoming_server_event_receiver.lock()?;
            let event = receiver.recv()?;

            info!(event = ?event, "handling event");

            match event {
                ServerEvent::IncomingMessage((id, message)) => match message {
                    Message::Client(_) => unreachable!(),
                    Message::Server(message) => match message {
                        ServerMessage::Hello(introduction) => {
                            let pdtcore_built_info = BuiltInfo::default();

                            let mut client_guard = self.clients.lock().unwrap();
                            let client = client_guard.get_mut(&id).unwrap();

                            if !pdtcore_built_info.compatible(&introduction.pdtcore_built_info) {
                                client.sender.send(ClientMessage::Goodbye.into()).unwrap();
                            }

                            client.pdtcore_built_info = Some(introduction.pdtcore_built_info);

                            client
                                .sender
                                .send(ClientMessage::RequestDeviceInfo.into())
                                .unwrap();
                        }
                        ServerMessage::Goodbye => todo!(),
                        ServerMessage::DeviceInfo(info) => {
                            let mut client_guard = self.clients.lock().unwrap();
                            let client = client_guard.get_mut(&id).unwrap();

                            client.device_info = Some(info);
                        }
                    },
                },
                ServerEvent::Unexpected(error) => error!(error = ?error),
            }
        }
    }

    #[instrument(skip(read))]
    fn handle_client_incoming_messages(
        id: Ulid,
        read: &mut dyn Read,
        sender: ServerSenderReference,
    ) -> Result<(), ReceiveError> {
        let mut ended = false;

        while !ended {
            let receive_result = Message::receive(read);

            let event = match receive_result {
                Ok(message) => {
                    if message == Message::from(ServerMessage::Goodbye) {
                        ended = true;
                    }
                    ServerEvent::IncomingMessage((id, message))
                }
                Err(error) => {
                    ended = true;
                    ServerEvent::Unexpected(error)
                }
            };

            let mut guard = sender.lock()?;

            let sender = &mut *guard;

            sender.send(event)?;
        }

        Ok(())
    }

    fn handle_client_outgoing_messages(id: Ulid, write: &mut dyn Write, receiver: ClientReceiver) {
        let mut ended = false;
        let id = id.to_string();

        while !ended {
            let receive_result = receiver.recv();

            let message = match receive_result {
                Ok(message) => message,
                Err(error) => {
                    error!(error =? error, client_id =? id, "receiving from outgoing client channel, will use end message");
                    ended = true;
                    ClientMessage::Goodbye.into()
                }
            };

            let send_result = message.send(write);

            match send_result {
                Ok(_) => {
                    info!(message =? message, client_id =? id, "sent")
                }
                Err(error) => {
                    ended = true;
                    error!(error =? error, client_id =? id, "writing outgoing client message")
                }
            }
        }
    }

    pub fn run(&mut self, tcp_listener: TcpListener) {
        let incoming_message_sender = self.incoming_server_event_sender.clone();
        let outgoing_message_senders = self.clients.clone();
        let client_ids = self.client_ids.clone();

        let handle_message_self = self.clone();

        std::thread::spawn(
            // TODO: figure out what to do if we stop handling messages due to errors
            //       should we resume/retry? exit? drop associated client?
            move || match handle_message_self.clone().handle_messages() {
                Ok(_) => {}
                Err(error) => {
                    error!(error =? error, "handle message")
                }
            },
        );

        std::thread::spawn(move || {
            for mut stream in tcp_listener.incoming().flatten() {
                trace!(stream = ?stream, "handle incoming tcp stream");

                let mut write_stream = match stream.try_clone() {
                    Ok(stream) => stream,
                    Err(error) => {
                        error!(error =? error, stream = ?stream, "failed copying tcp_stream for writing");
                        continue;
                    }
                };

                let id = Ulid::new();

                {
                    let mut guard = client_ids.lock().unwrap();

                    let client_ids = &mut *guard;

                    client_ids.push(id);
                }

                let sender = incoming_message_sender.clone();
                let (tx, rx) = mpsc::channel();

                let client = ServerClient {
                    id,
                    pdtcore_built_info: None,
                    sender: tx,
                    device_info: None,
                };

                std::thread::spawn(move || {
                    Server::handle_client_incoming_messages(id, &mut stream, sender)
                });

                let server_client_map = outgoing_message_senders.clone();

                std::thread::spawn(move || {
                    {
                        let mut guard = server_client_map.lock().unwrap();

                        let client_map = &mut *guard;

                        client_map.insert(id, client);
                    }
                    Server::handle_client_outgoing_messages(id, &mut write_stream, rx);
                    {
                        let mut guard = server_client_map.lock().unwrap();

                        let senders = &mut *guard;

                        senders.remove(&id)
                    }
                });
            }
        });
    }

    pub fn get_client_ids(&self) -> Result<Vec<Ulid>, PoisonError<MutexGuard<'_, Vec<Ulid>>>> {
        let guard = self.client_ids.lock()?;

        let client_ids = &*guard;

        Ok(client_ids.clone())
    }

    pub fn get_clients(&self) -> Vec<Client> {
        let client_guard = self.clients.lock().unwrap();
        let mut output = vec![];

        for (id, server_client) in client_guard.iter() {
            output.push(Client {
                id: id.to_string(),
                device_info: server_client.clone().device_info.unwrap_or_default(),
            })
        }

        output
    }

    pub fn send(&mut self, to: Ulid, message: Message) -> Result<(), SendError> {
        let Ok(mut clients_guard) = self.clients.lock() else {
            return Err(SendError::Deadlock);
        };

        let clients = &mut *clients_guard;

        let Some(client) = clients.get(&to) else {
            return Err(SendError::ClientNotFound);
        };

        let Ok(_) = client.sender.send(message) else {
            return Err(SendError::SendChannel);
        };

        Ok(())
    }
}
