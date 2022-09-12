use std::{
    future::Future,
    net::{Ipv6Addr, SocketAddr, SocketAddrV6},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

use tokio::net::{lookup_host, TcpListener, TcpStream, ToSocketAddrs};

pub type Error = Box<dyn std::error::Error + Send + Sync>;

pub async fn serve<A, H, Fut>(addr: A, mut handler: H) -> Result<(), Error>
where
    A: ToSocketAddrs,
    H: FnMut(Connection) -> Fut + Send + 'static + Copy,
    Fut: Future<Output = Result<(), Error>> + Send,
{
    let addr = lookup_host(addr)
        .await?
        .next()
        .unwrap_or_else(|| SocketAddr::V6(SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, 5555, 0, 0)));

    let listener = TcpListener::bind(addr).await?;
    println!("Listening on {addr}");

    let mut id = 0_usize;
    let running = Arc::new(AtomicUsize::new(0_usize));

    while let Ok((stream, addr)) = listener.accept().await {
        let connection = Connection::new(stream, addr, id);
        let running = running.clone();
        tokio::spawn(async move {
            let currently_running = running.fetch_add(1, Ordering::SeqCst) + 1;
            println!(">> [{id:>3}/{currently_running:>3}] {addr}");
            if let Err(err) = handler(connection).await {
                eprintln!("ERROR: {:?}", err);
            };
            println!("<< [{id:>3}/___] {addr}");
            running.fetch_sub(1, Ordering::SeqCst);
        });
        id = id.wrapping_add(1);
    }

    Ok(())
}

pub struct Connection {
    pub stream: TcpStream,
    pub addr: SocketAddr,
    pub id: usize,
}

impl Connection {
    pub fn new(stream: TcpStream, addr: SocketAddr, id: usize) -> Self {
        Self { stream, addr, id }
    }
}
