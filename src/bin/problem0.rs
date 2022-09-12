use protohackers::{serve, Error};

#[tokio::main]
async fn main() -> Result<(), Error> {
    serve("[::]:5555", |mut connection| async move {
        let (mut reader, mut writer) = connection.stream.split();
        let bytes_copied = tokio::io::copy(&mut reader, &mut writer).await?;
        println!("{} - {:>4}", connection.addr, bytes_copied);
        Ok(())
    })
    .await
}
