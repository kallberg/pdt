use std::{
    collections::HashMap,
    io::{Read, Write},
    net::TcpListener,
    sync::{mpsc, Arc, Mutex, MutexGuard, PoisonError},
};

use pdtcore::MessageProtocol;
use pdtcore::{Message, MessageProtocolError};
use tracing::*;

use ulid::Ulid;

type Shared<T> = Arc<Mutex<T>>;
type AddressedMessage = (Ulid, Message);
type ClientSender = mpsc::Sender<Message>;
type ClientReceiver = mpsc::Receiver<Message>;
type ServerSender = mpsc::Sender<ServerEvent>;
type ServerReceiver = mpsc::Receiver<ServerEvent>;
type SharedServerSender = Shared<ServerSender>;
type SharedServerReceiver = Shared<ServerReceiver>;

#[derive(Debug)]
pub enum SendError {
    ClientNotFound,
    SendChannelError,
    Deadlock,
}

#[derive(Debug)]
pub enum ServerEvent {
    Incoming(AddressedMessage),
    Unexpected(MessageProtocolError),
}

pub struct Server {
    incoming_server_event_sender: SharedServerSender,
    incoming_server_event_receiver: SharedServerReceiver,
    outgoing_message_senders: Shared<HashMap<Ulid, ClientSender>>,
    client_ids: Shared<Vec<Ulid>>,
}

impl Server {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();

        Self {
            incoming_server_event_sender: Arc::new(Mutex::new(tx)),
            incoming_server_event_receiver: Arc::new(Mutex::new(rx)),
            outgoing_message_senders: Arc::new(Mutex::new(HashMap::new())),
            client_ids: Arc::new(Mutex::new(vec![])),
        }
    }

    #[instrument(skip(receiver))]
    pub fn handle_message(receiver: SharedServerReceiver) {
        let event = receiver.lock().unwrap().recv().unwrap();

        info!(event = ?event, "handling event");

        match event {
            ServerEvent::Incoming((id, message)) => info!(id = ?id, message = ?message, "handled"),
            ServerEvent::Unexpected(error) => error!(error = ?error),
        }
    }

    #[instrument(skip(read))]
    fn handle_client_incoming_messages(id: Ulid, read: &mut dyn Read, sender: SharedServerSender) {
        let mut ended = false;

        while !ended {
            let receive_result = Message::receive(read);

            let event = match receive_result {
                Ok(message) => {
                    if message == Message::End {
                        ended = true;
                    }
                    ServerEvent::Incoming((id, message))
                }
                Err(error) => {
                    ended = true;
                    ServerEvent::Unexpected(error)
                }
            };

            let mut guard = sender.lock().unwrap();

            let sender = &mut *guard;

            sender.send(event).unwrap();
        }
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
                    Message::End
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
        let incoming_message_receiver = self.incoming_server_event_receiver.clone();
        let outgoing_message_senders = self.outgoing_message_senders.clone();
        let client_ids = self.client_ids.clone();

        std::thread::spawn(move || loop {
            Server::handle_message(incoming_message_receiver.clone())
        });

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

                //let write_copy = stream.try_clone().unwrap();

                let sender = incoming_message_sender.clone();
                let (tx, rx) = mpsc::channel();

                std::thread::spawn(move || {
                    Server::handle_client_incoming_messages(id, &mut stream, sender)
                });

                let outgoing_message_senders = outgoing_message_senders.clone();

                std::thread::spawn(move || {
                    {
                        let mut guard = outgoing_message_senders.lock().unwrap();

                        let senders = &mut *guard;

                        senders.insert(id, tx);
                    }
                    Server::handle_client_outgoing_messages(id, &mut write_stream, rx);
                    {
                        let mut guard = outgoing_message_senders.lock().unwrap();

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

    pub fn send(&mut self, to: Ulid, message: Message) -> Result<(), SendError> {
        let Ok(mut senders_guard) = self.outgoing_message_senders.lock() else {
            return Err(SendError::Deadlock);
        };

        let senders = &mut *senders_guard;

        let Some(sender) = senders.get(&to) else {
            return Err(SendError::ClientNotFound);
        };

        let Ok(_) = sender.send(message) else {
            return Err(SendError::SendChannelError);
        };

        Ok(())
    }
}
