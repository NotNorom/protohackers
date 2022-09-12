use std::collections::HashMap;
use std::io::ErrorKind;

use nom::character::complete::one_of;
use nom::combinator::eof;
use nom::number::complete::be_i32;
use nom::{Finish, IResult};

use protohackers::{serve, Error};
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufStream};

#[tokio::main]
async fn main() -> Result<(), Error> {
    serve("[::]:5555", |connection| async move {
        let mut stream = BufStream::new(connection.stream);

        let mut entries = HashMap::new();

        loop {
            let mut buffer = [0_u8; 9];
            let bytes_read = match stream.read_exact(&mut buffer).await {
                Ok(bytes_read) => bytes_read,
                Err(err) => match err.kind() {
                    ErrorKind::UnexpectedEof => break,
                    _ => Err(err)?,
                },
            };
            println!("   Bytes read: {bytes_read}");

            let message = Message::from_bytes(&buffer)?;
            println!("   {:?}", message);

            match message {
                Message::Insert { timestamp, price } => {
                    entries.insert(timestamp, price);
                }
                Message::Query { mintime, maxtime } => {
                    if mintime > maxtime {
                        stream.write_i32(0).await?;
                        stream.flush().await?;
                        continue;
                    }

                    let iter = entries
                        .iter()
                        .filter(|(&k, _)| mintime <= k && k <= maxtime);
                    let entry_count = iter.clone().count() as i64;
                    let sum: i64 = iter
                        .map(|(_, v)| *v)
                        .map(|v| v as i64)
                        .sum();

                    if entry_count == 0 {
                        stream.write_i32(0).await?;
                        stream.flush().await?;
                        continue;
                    }

                    let mean = (sum / entry_count) as i32;

                    println!("   {mean}");

                    stream.write_i32(mean).await?;
                    stream.flush().await?;
                }
            }
        }

        Ok(())
    })
    .await
}

#[derive(Debug)]
pub enum Message {
    Insert { timestamp: i32, price: i32 },
    Query { mintime: i32, maxtime: i32 },
}

impl Message {
    fn from_bytes(bytes: &[u8]) -> Result<Self, Error> {
        println!("   from_bytes:    {:02x?}", bytes);

        let message = match parse_message(bytes).finish() {
            Ok(ok) => ok.1,
            Err(err) => {
                return Err(Box::new(std::io::Error::new(
                    ErrorKind::Other,
                    err.code.description(),
                )));
            }
        };

        Ok(message)
    }
}

pub fn parse_message(i: &[u8]) -> IResult<&[u8], Message> {
    println!("   parse_message: {:02x?}", i);

    let (i, r#type) = one_of("QI")(i)?;
    let (i, param1) = be_i32(i)?;
    let (i, param2) = be_i32(i)?;
    let (_, _) = eof(i)?;

    let message = match r#type {
        'I' => Message::Insert {
            timestamp: param1,
            price: param2,
        },
        'Q' => Message::Query {
            mintime: param1,
            maxtime: param2,
        },
        _ => unreachable!("This is unreachable because of the above parser"),
    };

    Ok((i, message))
}
