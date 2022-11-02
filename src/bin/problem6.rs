use std::borrow::Borrow;
use std::convert::TryFrom;
use std::net::SocketAddr;
use std::time::Duration;

use anyhow::Result;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader, BufWriter, ReadBuf};
use tokio::net::tcp::ReadHalf;
use tokio::net::{TcpListener, TcpStream};
use tokio::select;
use tokio::sync::mpsc;
use tracing::{error, info, info_span, warn, Instrument, instrument};
use tracing_subscriber::prelude::__tracing_subscriber_SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use types::{ClientMessage, ClientState, Error, Heartbeat, Road, ServerMessage};

const PACKAGE_NAME: &str = env!("CARGO_CRATE_NAME");

mod types {
    use anyhow::{bail, Context};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt, BufReader, BufWriter},
        net::tcp::{OwnedReadHalf, OwnedWriteHalf},
    };

    #[derive(Debug)]
    pub enum ClientMessage {
        Plate(Plate),
        WantHeartbeat(WantHeartbeat),
        IAmCamera(IAmCamera),
        IAmDispatcher(IAmDispatcher),
    }

    impl ClientMessage {
        pub async fn from_bytes(
            reader: &mut BufReader<OwnedReadHalf>,
        ) -> anyhow::Result<ClientMessage> {
            let type_byte = reader.read_u8().await.context("reading type byte")?;

            match type_byte {
                0x20 => {
                    let plate_len = reader.read_u8().await?;

                    let mut plate = vec![0u8; plate_len as usize];
                    reader.read_exact(&mut plate).await?;

                    let timestamp = reader.read_u32().await?;

                    let plate = Plate {
                        plate: String::from_utf8_lossy(&plate).to_string(),
                        timestamp,
                    };

                    Ok(Self::Plate(plate))
                }
                0x40 => {
                    let interval = reader.read_u32().await?;
                    let want_heartbeat = WantHeartbeat { interval };
                    Ok(Self::WantHeartbeat(want_heartbeat))
                }
                0x80 => {
                    let road = reader.read_u16().await?;
                    let mile = reader.read_u16().await?;
                    let limit = reader.read_u16().await?;
                    let i_am_camera = IAmCamera { road, mile, limit };
                    Ok(Self::IAmCamera(i_am_camera))
                }
                0x81 => {
                    let numroads = reader.read_u8().await?;
                    let mut roads = Vec::with_capacity(numroads as usize);

                    for _ in 0..(numroads as usize) {
                        roads.push(reader.read_u16().await?);
                    }

                    let i_am_dispatcher = IAmDispatcher { numroads, roads };
                    Ok(Self::IAmDispatcher(i_am_dispatcher))
                }
                _ => bail!("Unexpected client message type byte {type_byte}"),
            }
        }
    }

    #[derive(Debug)]
    pub enum ServerMessage {
        Error(Error),
        Ticket(Ticket),
        Heartbeat(Heartbeat),
    }

    impl ServerMessage {
        pub async fn to_bytes(&self, writer: &mut BufWriter<OwnedWriteHalf>) -> anyhow::Result<()> {
            match self {
                Self::Error(error) => {
                    if error.msg.len() > 255 {
                        bail!("Error messages may not be longer than 255 characters");
                    }

                    writer.write_u8(0x10).await?;
                    writer.write_u8(error.msg.len() as u8).await?;
                    writer.write_all(error.msg.as_bytes()).await?;
                    Ok(())
                }
                Self::Ticket(ticket) => {
                    if ticket.plate.len() > 255 {
                        bail!("Plate string may not be longer than 255 characters");
                    }

                    writer.write_u8(0x21).await?;
                    writer.write_u8(ticket.plate.len() as u8).await?;
                    writer.write_all(ticket.plate.as_bytes()).await?;
                    writer.write_u16(ticket.road).await?;
                    writer.write_u16(ticket.mile1).await?;
                    writer.write_u32(ticket.timestamp1).await?;
                    writer.write_u16(ticket.mile2).await?;
                    writer.write_u32(ticket.timestamp2).await?;
                    writer.write_u16(ticket.speed).await?;
                    Ok(())
                }
                Self::Heartbeat(heartbeat) => {
                    writer.write_u8(0x41).await?;
                    Ok(())
                }
                _ => bail!("uhhh"),
            }
        }
    }

    #[derive(Debug)]
    pub struct Error {
        msg: String,
    }

    impl Error {
        pub fn with_msg(msg: impl ToString) -> Self {
            Self {
                msg: msg.to_string(),
            }
        }

        pub fn msg(&self) -> &String {
            &self.msg
        }
    }

    #[derive(Debug)]
    pub struct Plate {
        pub plate: String,
        pub timestamp: u32,
    }

    #[derive(Debug)]
    pub struct Ticket {
        pub plate: String,
        pub road: u16,
        pub mile1: u16,
        pub timestamp1: u32,
        pub mile2: u16,
        pub timestamp2: u32,
        /// (100x miles per hour)
        pub speed: u16,
    }

    /// The server must now send Heartbeat messages to this client at the given interval,
    /// which is specified in "deciseconds", of which there are 10 per second. (So an interval
    /// of "25" would mean a Heartbeat message every 2.5 seconds).
    /// The heartbeats help to assure the client that the server is still functioning,
    /// even in the absence of any other communication.
    /// An interval of 0 deciseconds means the client does not want to receive heartbeats (this is the default setting).
    #[derive(Debug)]
    pub struct WantHeartbeat {
        pub interval: u32,
    }

    #[derive(Debug)]
    pub struct Heartbeat();

    #[derive(Debug)]
    pub struct IAmCamera {
        pub road: u16,
        pub mile: u16,
        /// in miles per hour
        pub limit: u16,
    }

    #[derive(Debug)]
    pub struct IAmDispatcher {
        pub numroads: u8,
        pub roads: Vec<u16>,
    }

    #[derive(Debug)]
    pub enum ClientState {
        Camera { state: IAmCamera },
        Dispatcher { state: IAmDispatcher },
        Connecting,
    }

    #[derive(Debug, Clone, Copy)]
    pub struct Road {
        pub speed_limit: u16,
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG")
                .unwrap_or_else(|_| format!("{PACKAGE_NAME}=trace,tower_http=trace")),
        ))
        .with(tracing_subscriber::fmt::layer().compact())
        .init();

    let listener = TcpListener::bind("[::]:5555").await?;

    loop {
        let (stream, addr) = listener.accept().await?;
        let task = async move {
            if let Err(err) = handle_client(stream, addr).await {
                error!("Could not handle: {}, {:?}", err.root_cause(), err);
            }
        };
        tokio::spawn(task);
    }
}

#[instrument]
async fn handle_client(mut stream: TcpStream, addr: SocketAddr) -> Result<()> {
    let (reader, writer) = stream.into_split();
    let (mut reader, mut writer) = (BufReader::new(reader), BufWriter::new(writer));

    let (tx, mut rx) = mpsc::channel::<ServerMessage>(512);
    tokio::spawn(async move {
        while let Some(message) = rx.recv().await {
            if let Err(err) = message.to_bytes(&mut writer).await {
                error!("Could not send message: {}, {:?}", err.root_cause(), err);
            }
        }
    });

    let mut heartbeat_running = false;
    let mut client_state = ClientState::Connecting;

    loop {
        let client_msg = types::ClientMessage::from_bytes(&mut reader).await?;
        let tx = tx.clone();

        match client_msg {
            ClientMessage::Plate(plate) => match client_state {
                ClientState::Camera { ref state } => {
                    info!("Received {plate:?} on {state:?}");
                }
                _ => {
                    let _ = tx
                        .send(ServerMessage::Error(Error::with_msg(
                            "You are not a camera",
                        )))
                        .await?;
                    break;
                }
            },
            ClientMessage::WantHeartbeat(hearbeat) => {
                if heartbeat_running {
                    let _ = tx
                        .send(ServerMessage::Error(Error::with_msg(
                            "Cannot request multiple heartbeats",
                        )))
                        .await;
                    break;
                }

                let duration = hearbeat.interval * 100;
                let duration = Duration::from_millis(duration as u64);

                tokio::spawn(async move {
                    loop {
                        if let Err(_) = tx.send(ServerMessage::Heartbeat(Heartbeat())).await {
                            break;
                        };
                        tokio::time::sleep(duration).await;
                    }
                });

                heartbeat_running = true;
            }
            ClientMessage::IAmCamera(i_am_camera) => match client_state {
                ClientState::Camera { .. } => {
                    let _ = tx
                        .send(ServerMessage::Error(Error::with_msg(
                            "You are already a camera",
                        )))
                        .await?;
                    break;
                }
                ClientState::Dispatcher { .. } => {
                    let _ = tx
                        .send(ServerMessage::Error(Error::with_msg(
                            "You are a camera, not a dispatcher",
                        )))
                        .await?;
                    break;
                }
                ClientState::Connecting => {
                    client_state = ClientState::Camera { state: i_am_camera };
                    info!("Client has identified as camera: {client_state:?}");
                }
            },
            ClientMessage::IAmDispatcher(i_am_dispatcher) => match client_state {
                ClientState::Camera { .. } => {
                    let _ = tx
                        .send(ServerMessage::Error(Error::with_msg(
                            "You are a dispatcher, not a camera",
                        )))
                        .await?;
                    break;
                }
                ClientState::Dispatcher { .. } => {
                    let _ = tx
                        .send(ServerMessage::Error(Error::with_msg(
                            "You are already a dispatcher",
                        )))
                        .await?;
                    break;
                }
                ClientState::Connecting => {
                    client_state = ClientState::Dispatcher {
                        state: i_am_dispatcher,
                    };
                    info!("Client has identified as dispatcher: {client_state:?}");
                }
            },
        }
    }

    info!("Disconnect");
    Ok(())
}

async fn handle_tickets() -> Result<()> {
    let mut roads = [Road { speed_limit: 0 }; u16::MAX as usize];

    Ok(())
}
