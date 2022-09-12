use primes::is_prime;
use protohackers::{serve, Error};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};

#[tokio::main]
async fn main() -> Result<(), Error> {
    serve("[::]:5555", |mut connection| async move {
        let (reader, writer) = connection.stream.split();
        let reader = BufReader::new(reader);
        let mut writer = BufWriter::new(writer);

        let mut lines = reader.lines();

        while let Some(line) = lines.next_line().await? {
            match serde_json::from_str::<Request>(&line) {
                Ok(request) => {
                    if !request.method_is_valid() {
                        println!("   Invalid method: {}", request.method);
                        writer.write_all(b"malformed").await?;
                        writer.flush().await?;
                        // disconnect
                        break;
                    }
                    let response = Response::new(request.is_prime());
                    let mut response_bytes = serde_json::to_vec(&response)?;
                    response_bytes.push(b'\n');

                    println!(
                        "   Valid json. Request: {:?}. Responding with: {}",
                        request,
                        String::from_utf8_lossy(&response_bytes)
                    );
                    writer.write_all(&response_bytes).await?;
                    writer.flush().await?;
                }
                Err(err) => {
                    println!("   Malformed: {}", err);
                    writer.write_all(b"malformed").await?;
                    writer.flush().await?;
                    // disconnect
                    break;
                }
            }
        }

        Ok(())
    })
    .await
}

#[derive(Serialize)]
struct Response {
    method: &'static str,
    prime: bool,
}

impl Response {
    pub fn new(prime: bool) -> Self {
        Self {
            method: "isPrime",
            prime,
        }
    }
}

#[derive(Deserialize, Debug)]
struct Request {
    method: String,
    number: f64,
}

impl Request {
    fn method_is_valid(&self) -> bool {
        self.method == "isPrime"
    }

    fn is_prime(&self) -> bool {
        if self.number.fract() != 0.0 {
            return false;
        }

        if !self.number.is_normal() {
            return false;
        }

        let maybe_prime: u64 = unsafe { self.number.trunc().to_int_unchecked() };

        is_prime(maybe_prime)
    }
}
