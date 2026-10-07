//! Versioned restricted helper protocol. Maintain one socket for the lease lifetime.
use serde::{Deserialize, Serialize};
pub const SOCKET_PATH: &str = "/run/linuxdrop/netd.sock";
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    HostP2p {
        lease_id: String,
        #[serde(default)]
        auth: linuxdrop_network::P2pHostAuth,
    },
    CancelP2p {
        lease_id: String,
    },
    JoinP2p {
        lease_id: String,
        peer_name: String,
        pin: String,
        frequency: u32,
    },
    LeaveP2p {
        lease_id: String,
    },
    Reserve {
        radio_id: String,
    },
    Diagnose {
        radio_id: String,
        channel: u16,
    },
    RecoveryStatus,
    RetryRecovery,
    Acquire {
        radio_id: String,
        channel: u16,
    },
    AcquireAwdl {
        radio_id: String,
        channel: u16,
    },
    SetChannel {
        lease_id: String,
        channel: u16,
    },
    Release {
        lease_id: String,
    },
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
    #[serde(default)]
    pub kind: LeaseKind,
    #[serde(default)]
    pub connection_uuid: Option<String>,
    #[serde(default)]
    pub p2p_group: Option<linuxdrop_network::p2p::GroupIdentity>,
    /// Persisted before formation; cleared only after proven settlement.
    #[serde(default)]
    pub p2p_pending: bool,
    #[serde(default)]
    pub p2p_recovery: Option<P2pRecovery>,
    #[serde(default)]
    pub direct_capabilities: linuxdrop_network::DirectWifiCapabilities,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct P2pRecovery {
    pub formation: linuxdrop_network::p2p::FormationIdentity,
    pub kernel_interfaces: std::collections::BTreeMap<String, u32>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LeaseKind {
    #[default]
    Monitor,
    DirectWifi,
}
pub use linuxdrop_hardware::{DiagnosticReport, DiagnosticStep, RecoveryIssue};
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Response {
    P2pHosted {
        #[serde(default)]
        device_name: Option<String>,
        interface: String,
        ssid: String,
        password: String,
        frequency: u16,
        ipv4_address: std::net::Ipv4Addr,
        ipv6_address: Option<std::net::Ipv6Addr>,
    },
    P2pJoined {
        interface: String,
        ipv4_address: Option<std::net::Ipv4Addr>,
        #[serde(default)]
        ipv6_address: Option<std::net::Ipv6Addr>,
    },
    Diagnostic {
        report: DiagnosticReport,
    },
    Recovery {
        issues: Vec<RecoveryIssue>,
        recent_errors: Vec<String>,
    },
    Acquired {
        lease: Box<Lease>,
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

impl std::fmt::Debug for Response {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::P2pHosted {
                interface,
                frequency,
                ipv4_address,
                ipv6_address,
                ..
            } => f
                .debug_struct("P2pHosted")
                .field("interface", interface)
                .field("frequency", frequency)
                .field("ipv4_address", ipv4_address)
                .field("ipv6_address", ipv6_address)
                .finish_non_exhaustive(),
            Self::P2pJoined {
                interface,
                ipv4_address,
                ipv6_address,
            } => f
                .debug_struct("P2pJoined")
                .field("interface", interface)
                .field("ipv4_address", ipv4_address)
                .field("ipv6_address", ipv6_address)
                .finish(),
            Self::Diagnostic { report } => f
                .debug_struct("Diagnostic")
                .field("report", report)
                .finish(),
            Self::Recovery {
                issues,
                recent_errors,
            } => f
                .debug_struct("Recovery")
                .field("issues", issues)
                .field("recent_errors", recent_errors)
                .finish(),
            Self::Acquired { lease } => f.debug_struct("Acquired").field("lease", lease).finish(),
            Self::Ok => f.write_str("Ok"),
            Self::State {
                leases,
                recovery_errors,
            } => f
                .debug_struct("State")
                .field("leases", leases)
                .field("recovery_errors", recovery_errors)
                .finish(),
            Self::Error { message } => f.debug_struct("Error").field("message", message).finish(),
        }
    }
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
        Ok(Self::from_stream(stream))
    }
    /// Use an already connected Unix socket with the same framed helper protocol.
    pub fn from_stream(stream: tokio::net::UnixStream) -> Self {
        let (reader, writer) = stream.into_split();
        Self {
            reader: tokio::io::BufReader::new(reader),
            writer,
        }
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
