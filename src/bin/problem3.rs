use std::collections::HashSet;

use protohackers::Error;
use tokio::net::TcpListener;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::select;
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio::task::JoinHandle;

#[derive(Debug, Clone, PartialEq)]
struct Username(String);
impl Username {
    pub fn get(&self) -> &str {
        self.0.as_str()
    }
    pub fn new(name: &str) -> Result<Self, &'static str> {
        if name.is_empty() {
            return Err("name empty");
        }
        if name.chars().any(|ch| !ch.is_alphanumeric()) {
            return Err("name contains non-alphanumeric characters");
        }

        Ok(Username(name.to_string()))
    }
}

#[derive(Debug, Clone)]
struct Content(String);
impl Content {
    pub fn get(&self) -> &str {
        self.0.as_str()
    }
    pub fn new(content: &str) -> Self {
        Content(content.to_string())
    }
}

#[derive(Debug)]
enum IncomingEvent {
    Join(Username, oneshot::Sender<String>),
    Part(Username),
    Message(Username, Content),
}

#[derive(Debug, Clone)]
enum OutgoingEvent {
    Join(Username),
    Part(Username),
    Message(Username, Content),
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    let listener = TcpListener::bind("[::]:5555").await?;

    let (incoming_event_tx, mut incoming_event_rx) = mpsc::channel::<IncomingEvent>(128);
    let (outgoing_event_tx, _) = broadcast::channel::<OutgoingEvent>(128);
    let outgoing_event_tx_manager = outgoing_event_tx.clone();

    let _: JoinHandle<Result<(), Error>> = tokio::spawn(async move {
        let mut usernames = HashSet::<String>::new();

        while let Some(event) = incoming_event_rx.recv().await {
            match event {
                IncomingEvent::Join(username, reply) => {
                    let current_users: Vec<String> = usernames.iter().cloned().collect();
                    let current_users = current_users.join(", ");
                    reply.send(current_users).unwrap();

                    usernames.insert(username.get().to_string());
                    let _ = outgoing_event_tx_manager.send(OutgoingEvent::Join(username));
                }
                IncomingEvent::Part(username) => {
                    usernames.remove(username.get());
                    let _ = outgoing_event_tx_manager.send(OutgoingEvent::Part(username));
                }
                IncomingEvent::Message(username, content) => {
                    let _ =
                        outgoing_event_tx_manager.send(OutgoingEvent::Message(username, content));
                }
            };
        }
        Ok(())
    });

    loop {
        let (mut stream, _) = listener.accept().await?;
        let incoming_event_tx = incoming_event_tx.clone();
        let mut outgoing_event_rx = outgoing_event_tx.subscribe();

        let _: JoinHandle<Result<(), Error>> = tokio::spawn(async move {
            let (reader, writer) = stream.split();
            let (mut reader, mut writer) = (BufReader::new(reader), BufWriter::new(writer));

            let username = {
                let mut username = String::with_capacity(16);

                writer.write_all(b"name?\n").await.unwrap();
                writer.flush().await.unwrap();

                if reader.read_line(&mut username).await.unwrap() == 0 {
                    return Ok(());
                }
                let username = match Username::new(username.trim()) {
                    Ok(username) => username,
                    Err(err) => {
                        eprintln!("Error: {err}");
                        return Ok(());
                    }
                };

                let (user_list_sender, user_list_reply) = oneshot::channel();

                incoming_event_tx
                    .send(IncomingEvent::Join(username.clone(), user_list_sender))
                    .await?;
                let user_list = user_list_reply.await?;

                let message = format!("* LIST: {user_list}\n");
                let _ = writer.write_all(message.as_bytes()).await;
                let _ = writer.flush().await;

                username
            };

            let mut lines = reader.lines();
            outgoing_event_rx = outgoing_event_rx.resubscribe();

            loop {
                select! {
                    Ok(maybe_incoming) = lines.next_line() => {
                        if let Some(incoming) = maybe_incoming {
                            let event = IncomingEvent::Message(username.clone(), Content::new(&incoming));
                            incoming_event_tx.send(event).await?;
                        } else {
                            break;
                        }
                    }

                    Ok(outgoing_event) = outgoing_event_rx.recv() => {
                        match outgoing_event {
                            OutgoingEvent::Join(author) => {
                                if author == username {
                                    continue
                                }
                                let message = format!("* JOIN: {}\n", author.get());
                                let _ = writer.write_all(message.as_bytes()).await;
                                let _ = writer.flush().await;
                            },
                            OutgoingEvent::Part(author) => {
                                if author == username {
                                    continue
                                }
                                let message = format!("* PART: {}\n", author.get());
                                let _ = writer.write_all(message.as_bytes()).await;
                                let _ = writer.flush().await;
                            },
                            OutgoingEvent::Message(author, content) => {
                                if author == username {
                                    continue
                                }
                                let message = format!("[{}] {}\n", author.get(), content.get());
                                let _ = writer.write_all(message.as_bytes()).await;
                                let _ = writer.flush().await;
                            },
                        };
                    }
                    else => {
                        break
                    }
                }
            }
            let _ = incoming_event_tx.send(IncomingEvent::Part(username)).await;

            Ok(())
        });
    }
}
