use std::collections::HashMap;

use anyhow::Result;
use tokio::net::UdpSocket;

fn main() -> Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(try_main())?;
    Ok(())
}

async fn try_main() -> Result<()> {
    let socket = UdpSocket::bind("[::]:5555").await?;
    let mut database = HashMap::with_capacity(10_000);
    let mut buffer = [0_u8; 1024];

    loop {
        let (bytes_read, addr) = socket.recv_from(&mut buffer).await?;
        let data = &buffer[0..bytes_read];

        if let Some(first_space_idx) = data.iter().position(|&byte| byte == b'=') {
            // Insert
            let (key, value) = data.split_at(first_space_idx);
            database.insert(key.to_owned(), value.to_owned());
        } else {
            // Retrieve
            let key = data;
            if key == b"version" {
                socket.send_to(b"version=norom - v69.420", addr).await?;
                continue;
            }
            let mut value = database.get(key).cloned().unwrap_or_default();

            let mut reply = key.to_vec();
            reply.append(&mut value);
            socket.send_to(&reply, addr).await?;
        }
    }
}
