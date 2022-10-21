use std::net::SocketAddr;

use anyhow::Result;
use tokio::net::{TcpListener, TcpStream};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::select;
use tracing::{error, info, info_span, Instrument, warn};
use tracing_subscriber::prelude::__tracing_subscriber_SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

const TONY: &str = "7YWHMfk9JZe0LM0g1ZauHuiSxhI";
const TARGET: &str = "[2a03:b0c0:1:d0::116a:8001]:16963";
const PACKAGE_NAME: &str = env!("CARGO_CRATE_NAME");

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
    let target: SocketAddr = TARGET.parse().unwrap();

    loop {
        let (stream, addr) = listener.accept().await?;
        let task = async move {
            if let Err(err) = forward(stream, addr, target).await {
                error!("{err}");
            }
        };
        tokio::spawn(task);
    }
}

fn do_the_boguscoin_rewrite(input: &str) -> String {
    let input = input.strip_suffix('\n').unwrap_or(input);

    let mut words: Vec<&str> = input.split_ascii_whitespace().collect();

    for word in words.iter_mut() {
        if word.starts_with('7')
            && word.len() >= 26
            && word.len() <= 35
            && word.chars().all(|char| char.is_alphanumeric())
        {
            *word = TONY;
        }
    }

    let rewritten = words.join(" ");
    info!("rewritten {input:?}  -->  {rewritten:?}");
    rewritten
}

async fn forward(
    mut inbound: TcpStream,
    original_addr: SocketAddr,
    target_addr: SocketAddr,
) -> anyhow::Result<()> {
    info!("Accept - {original_addr:?} -> {target_addr:?}");

    let mut outbound = TcpStream::connect(target_addr).await?;

    let (inbound_r, inbound_w) = inbound.split();
    let (mut inbound_r, mut inbound_w) = (BufReader::new(inbound_r), BufWriter::new(inbound_w));

    let (outbound_r, outbound_w) = outbound.split();
    let (mut outbound_r, mut outbound_w) = (BufReader::new(outbound_r), BufWriter::new(outbound_w));

    let span = info_span!("o2t", "{original_addr:?} -> {target_addr:?}");
    let original_to_target = async {
        let mut line = String::with_capacity(1024);

        loop {
            line.clear();
            let bytes_read = inbound_r.read_line(&mut line).await?;
            if bytes_read <= 1 {
                warn!("EOF");
                break;
            }

            if !line.ends_with('\n') {
                warn!("Disconnected without sending \\n");
                break;
            }

            let line = do_the_boguscoin_rewrite(&line);
            outbound_w
                .write_all(format!("{}\n", line).as_bytes())
                .await?;
            outbound_w.flush().await?;
        }
        info!("Disconnect o2t half");
        Ok::<(), anyhow::Error>(())
    }
    .instrument(span);

    let span = info_span!("t2o", "{target_addr:?} -> {original_addr:?}");
    let target_to_original = async {
        let mut line = String::with_capacity(1024);

        loop {
            line.clear();
            let bytes_read = outbound_r.read_line(&mut line).await?;
            if bytes_read <= 1 {
                warn!("EOF");
                break;
            }

            if !line.ends_with('\n') {
                warn!("Disconnected without sending \\n");
                break;
            }

            let line = do_the_boguscoin_rewrite(&line);
            inbound_w
                .write_all(format!("{}\n", line).as_bytes())
                .await?;
            inbound_w.flush().await?;
        }
        info!("Disconnect t2o half");
        Ok::<(), anyhow::Error>(())
    }
    .instrument(span);

    select! {
        _ = original_to_target => {},
        _ = target_to_original => {},
    }

    info!("Disconnect");

    Ok(())
}
