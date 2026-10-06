use std::{
    collections::HashMap,
    net::IpAddr,
    sync::Mutex,
    time::{Duration, Instant},
};

/// Bound both per-source request work and attacker-controlled tracking memory.
pub(crate) struct RequestGate {
    entries: Mutex<HashMap<IpAddr, (Instant, u32)>>,
    limit: u32,
    period: Duration,
}
impl RequestGate {
    pub(crate) fn new(limit: u32, period: Duration) -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            limit,
            period,
        }
    }
    pub(crate) fn allow(&self, ip: IpAddr) -> bool {
        self.allow_at(ip, Instant::now())
    }
    fn allow_at(&self, ip: IpAddr, now: Instant) -> bool {
        let mut entries = self.entries.lock().unwrap();
        entries.retain(|_, (started, _)| now.duration_since(*started) < self.period);
        if entries.len() >= 256 && !entries.contains_key(&ip) {
            return false;
        }
        let (_, count) = entries.entry(ip).or_insert((now, 0));
        if *count >= self.limit {
            return false;
        }
        *count += 1;
        true
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounds_sources_and_resets_after_window() {
        let gate = RequestGate::new(2, Duration::from_secs(60));
        let now = Instant::now();
        let ip = "192.0.2.1".parse().unwrap();
        assert!(gate.allow_at(ip, now));
        assert!(gate.allow_at(ip, now));
        assert!(!gate.allow_at(ip, now));
        for host in 1..=255 {
            assert!(gate.allow_at(IpAddr::V4(std::net::Ipv4Addr::new(198, 51, 100, host)), now));
        }
        assert!(!gate.allow_at("203.0.113.1".parse().unwrap(), now));
        assert!(gate.allow_at(ip, now + Duration::from_secs(60)));
    }
}
