//! Versioned restricted helper protocol. Maintain one socket for the lease lifetime.
use serde::{Deserialize, Serialize};
pub const SOCKET_PATH: &str = "/run/linuxdrop/netd.sock";
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    Acquire { radio_id: String, channel: u16 },
    AcquireAwdl { radio_id: String, channel: u16 },
    SetChannel { lease_id: String, channel: u16 },
    Release { lease_id: String },
    Status,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Lease {
    pub id: String,
    pub uid: u32,
    pub phy: String,
    pub interface: String,
    pub channel: u16,
    pub boot_id: String,
    pub awdl_interface: Option<String>,
    #[serde(default)]
    pub allowed_frequencies: Vec<u32>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Response {
    Acquired {
        lease: Lease,
    },
    Ok,
    State {
        leases: Vec<Lease>,
        recovery_errors: Vec<String>,
    },
    Error {
        message: String,
    },
}

#[cfg(unix)]
pub struct Client {
    reader: tokio::io::BufReader<tokio::net::unix::OwnedReadHalf>,
    writer: tokio::net::unix::OwnedWriteHalf,
}
#[cfg(unix)]
impl Client {
    pub async fn connect() -> std::io::Result<Self> {
        let stream = tokio::net::UnixStream::connect(SOCKET_PATH).await?;
        let (reader, writer) = stream.into_split();
        Ok(Self {
            reader: tokio::io::BufReader::new(reader),
            writer,
        })
    }
    pub async fn request(&mut self, request: &Request) -> std::io::Result<Response> {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
        let mut bytes = serde_json::to_vec(request)?;
        bytes.push(b'\n');
        self.writer.write_all(&bytes).await?;
        let mut line = String::new();
        self.reader.read_line(&mut line).await?;
        serde_json::from_str(&line).map_err(std::io::Error::other)
    }
}
