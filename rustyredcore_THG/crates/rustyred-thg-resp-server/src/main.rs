mod protocol;

use std::sync::{Arc, Mutex};

use protocol::{execute_resp_command, RespValue};
use rustyred_thg_core::OrderedIndexRegistry;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let addr =
        std::env::var("RUSTYRED_THG_RESP_ADDR").unwrap_or_else(|_| "127.0.0.1:6380".to_string());
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    let indexes = Arc::new(Mutex::new(OrderedIndexRegistry::new()));
    eprintln!("RUSTYRED_THG_RESP_READY {}", addr);
    loop {
        let (socket, _peer) = listener.accept().await?;
        let indexes = Arc::clone(&indexes);
        tokio::spawn(async move {
            if let Err(error) = serve_connection(socket.into_split(), indexes).await {
                eprintln!("RUSTYRED_THG_RESP_CONNECTION_ERROR {error}");
            }
        });
    }
}

async fn serve_connection(
    (reader, mut writer): (OwnedReadHalf, OwnedWriteHalf),
    indexes: Arc<Mutex<OrderedIndexRegistry>>,
) -> std::io::Result<()> {
    let mut reader = BufReader::new(reader);
    while let Some(parts) = read_command(&mut reader).await? {
        let response = {
            let mut indexes = indexes
                .lock()
                .expect("RESP ordered index registry poisoned");
            execute_resp_command(&mut indexes, &parts)
        };
        writer.write_all(&response.encode()).await?;
    }
    Ok(())
}

async fn read_command(
    reader: &mut BufReader<OwnedReadHalf>,
) -> std::io::Result<Option<Vec<String>>> {
    let mut line = String::new();
    if reader.read_line(&mut line).await? == 0 {
        return Ok(None);
    }
    let line = trim_crlf(&line);
    if let Some(count) = line.strip_prefix('*') {
        let count = parse_len(count)?;
        let mut parts = Vec::with_capacity(count);
        for _ in 0..count {
            let mut len_line = String::new();
            if reader.read_line(&mut len_line).await? == 0 {
                return Ok(None);
            }
            let Some(raw_len) = trim_crlf(&len_line).strip_prefix('$') else {
                return Ok(Some(vec!["__RESP_ERROR__".to_string()]));
            };
            let len = parse_len(raw_len)?;
            let mut bytes = vec![0_u8; len + 2];
            reader.read_exact(&mut bytes).await?;
            bytes.truncate(len);
            parts.push(String::from_utf8_lossy(&bytes).to_string());
        }
        return Ok(Some(parts));
    }
    Ok(Some(
        line.split_whitespace()
            .map(str::to_string)
            .collect::<Vec<_>>(),
    ))
}

fn parse_len(raw: &str) -> std::io::Result<usize> {
    raw.parse::<usize>().map_err(|error| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("invalid RESP length {raw}: {error}"),
        )
    })
}

fn trim_crlf(value: &str) -> &str {
    value.trim_end_matches(['\r', '\n'])
}

#[allow(dead_code)]
fn error_response(message: impl Into<String>) -> RespValue {
    RespValue::Error(message.into())
}
