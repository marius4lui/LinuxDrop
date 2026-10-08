//! Joint, passive allocation. A radio is exclusively assigned to one backend.
//! The privileged helper still revalidates current ownership before acquisition.
use crate::{select_radio, Candidate, Inventory, Radio, SelectionRequest};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug)]
pub struct Request {
    pub airdrop: bool,
    pub quickshare: bool,
    pub preferred: Option<String>,
    pub prefer_usb: bool,
    pub auto_use_usb: bool,
    /// Either stable IDs or kernel phy names, including failed-cleanup leases.
    pub leased: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AwdlChoice {
    pub radio_id: String,
    pub channel: u16,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Plan {
    pub airdrop: Option<AwdlChoice>,
    pub quickshare: Option<String>,
    pub direct_candidates: Vec<Candidate>,
}

fn automatic_allowed(radio: &Radio, request: &Request) -> bool {
    radio.bus != "usb" || request.auto_use_usb || request.preferred.as_ref() == Some(&radio.id)
}

fn direct_candidates(inventory: &Inventory, request: &Request) -> Vec<Candidate> {
    let mut candidates: Vec<_> = inventory
        .radios
        .iter()
        .map(|radio| {
            let mut exclusions = Vec::new();
            if !request.quickshare {
                exclusions.push("Quick Share is disabled".into());
            }
            if !automatic_allowed(radio, request) {
                exclusions.push("automatic USB use is disabled".into());
            }
            if radio.rfkill {
                exclusions.push("radio is blocked by rfkill".into());
            }
            if radio.driver.is_none() {
                exclusions.push("no driver detected".into());
            }
            if radio.protected
                || inventory.interfaces.iter().any(|interface| {
                    interface.phy.as_deref() == Some(&radio.phy) && interface.in_use()
                })
            {
                exclusions.push("radio is active or carries a default route".into());
            }
            if request.leased.contains(&radio.id) || request.leased.contains(&radio.phy) {
                exclusions.push("radio is already leased".into());
            }
            if !radio.modes.iter().any(|mode| mode == "managed") {
                exclusions.push("managed station mode is unavailable".into());
            }
            if !radio.channels.iter().any(|channel| !channel.disabled) {
                exclusions.push("no enabled Wi-Fi channel".into());
            }
            if !inventory.interfaces.iter().any(|interface| {
                interface.phy.as_deref() == Some(&radio.phy) && !interface.in_use()
            }) {
                exclusions.push("no idle interface on selected radio".into());
            }
            let score = if request.preferred.as_ref() == Some(&radio.id) {
                1000
            } else {
                0
            } + if request.prefer_usb && radio.bus == "usb" {
                50
            } else {
                0
            } + if radio.modes.iter().any(|mode| mode == "P2P-GO") {
                20
            } else {
                0
            } + if radio.modes.iter().any(|mode| mode == "P2P-client") {
                10
            } else {
                0
            } + if radio.modes.iter().any(|mode| mode == "AP") {
                5
            } else {
                0
            };
            Candidate {
                id: radio.id.clone(),
                score,
                exclusions,
            }
        })
        .collect();
    candidates.sort_by(|a, b| b.score.cmp(&a.score).then(a.id.cmp(&b.id)));
    candidates
}

/// Maximize usable transports first, then prefer AirDrop when one radio is
/// shared (Quick Share retains LAN), then apply explicit preference and quality.
/// Stable candidate order breaks ties independently of inventory enumeration.
pub fn allocate(inventory: &Inventory, request: &Request) -> Plan {
    let direct_candidates = direct_candidates(inventory, request);
    let direct: Vec<_> = std::iter::once(None)
        .chain(
            direct_candidates
                .iter()
                .filter(|candidate| candidate.exclusions.is_empty())
                .map(Some),
        )
        .collect();
    let mut awdl = Vec::new();
    if request.airdrop {
        for (channel_rank, channel) in [44, 6, 149].into_iter().enumerate() {
            let decision = select_radio(
                inventory,
                &SelectionRequest {
                    preferred: request.preferred.clone(),
                    leased: request.leased.clone(),
                    prefer_usb: request.prefer_usb,
                    channel: Some(channel),
                    require_tested_awdl: false,
                },
            );
            for candidate in decision
                .candidates
                .into_iter()
                .filter(|candidate| candidate.exclusions.is_empty())
            {
                let radio = inventory
                    .radios
                    .iter()
                    .find(|radio| radio.id == candidate.id)
                    .unwrap();
                if automatic_allowed(radio, request)
                    && !inventory.interfaces.iter().any(|interface| {
                        interface.phy.as_deref() == Some(&radio.phy) && interface.in_use()
                    })
                {
                    awdl.push((
                        AwdlChoice {
                            radio_id: candidate.id,
                            channel,
                        },
                        candidate.score,
                        channel_rank,
                    ));
                }
            }
        }
    }
    let mut best = None;
    let mut chosen = (None, None);
    for a in std::iter::once(None).chain(awdl.iter().map(Some)) {
        for q in &direct {
            if let (Some((a, _, _)), Some(q)) = (a, q) {
                let a_phy = &inventory
                    .radios
                    .iter()
                    .find(|r| r.id == a.radio_id)
                    .unwrap()
                    .phy;
                let q_phy = &inventory.radios.iter().find(|r| r.id == q.id).unwrap().phy;
                if a_phy == q_phy {
                    continue;
                }
            }
            let score = (
                usize::from(a.is_some()) + usize::from(q.is_some()),
                a.is_some(),
                a.is_some_and(|(a, _, _)| request.preferred.as_ref() == Some(&a.radio_id)),
                a.map_or(0, |(_, score, _)| *score) + q.map_or(0, |q| q.score),
                std::cmp::Reverse(a.map_or(0, |(_, _, rank)| *rank)),
            );
            if best.is_none_or(|best| score > best) {
                best = Some(score);
                chosen = (
                    a.map(|(choice, _, _)| choice.clone()),
                    q.map(|q| q.id.clone()),
                );
            }
        }
    }
    Plan {
        airdrop: chosen.0,
        quickshare: chosen.1,
        direct_candidates,
    }
}

/// Remembers a topology change while transfers or a restart are active.
/// Caller passes only roles missing a lease and acknowledges a queued restart.
#[derive(Default)]
pub struct HotplugAllocation {
    observed: Option<(Vec<Radio>, Vec<crate::NetworkInterface>)>,
    pending: bool,
    attempted: std::collections::HashSet<String>,
    proposed: Vec<String>,
}
impl HotplugAllocation {
    pub fn observe(&mut self, inventory: &Inventory, missing: &Request, busy: bool) -> bool {
        // A failed engine can create/remove its own temporary interfaces. Do
        // not turn those udev events into unlimited acquisition/restart loops.
        // Unplug or rfkill resets the attachment's automatic attempt budget;
        // explicit user-requested restarts do not go through this gate.
        self.attempted.retain(|id| {
            inventory
                .radios
                .iter()
                .any(|radio| &radio.id == id && !radio.rfkill)
        });
        let topology = (inventory.radios.clone(), inventory.interfaces.clone());
        if self.observed.as_ref() != Some(&topology) {
            self.observed = Some(topology);
            self.pending = true;
        }
        if !self.pending || busy {
            return false;
        }
        let plan = allocate(inventory, missing);
        self.proposed = plan
            .airdrop
            .map(|choice| choice.radio_id)
            .into_iter()
            .chain(plan.quickshare)
            .collect();
        if self.proposed.iter().any(|id| !self.attempted.contains(id)) {
            return true;
        }
        self.pending = false;
        false
    }
    pub fn queued(&mut self) {
        self.attempted.extend(self.proposed.drain(..));
        self.pending = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Capability, CapabilityValue, Channel, NetworkInterface};
    fn radio(id: &str, monitor: bool) -> Radio {
        Radio {
            id: id.into(),
            phy: format!("phy-{id}"),
            bus: "usb".into(),
            device_path: String::new(),
            vendor_id: None,
            product_id: None,
            serial: None,
            driver: Some("fixture".into()),
            kernel: String::new(),
            driver_details: Default::default(),
            bands: vec![],
            combinations: vec![],
            evidence_profile: Default::default(),
            rfkill: false,
            interfaces: vec![id.into()],
            channels: vec![Channel {
                number: 6,
                frequency_mhz: 2437,
                ..Default::default()
            }],
            modes: vec!["managed".into(), "P2P-GO".into(), "P2P-client".into()],
            interface_combinations: vec![],
            monitor: Capability {
                value: if monitor {
                    CapabilityValue::Yes
                } else {
                    CapabilityValue::No
                },
                source: "fixture".into(),
                evidence: String::new(),
            },
            management_injection: Capability::unknown("fixture"),
            data_injection: Capability::unknown("fixture"),
            awdl: Capability::unknown("fixture"),
            protected: false,
        }
    }
    fn inventory(radios: Vec<Radio>) -> Inventory {
        let interfaces = radios
            .iter()
            .map(|radio| NetworkInterface {
                name: radio.id.clone(),
                phy: Some(radio.phy.clone()),
                ..Default::default()
            })
            .collect();
        Inventory {
            radios,
            interfaces,
            ..Default::default()
        }
    }
    fn request() -> Request {
        Request {
            airdrop: true,
            quickshare: true,
            preferred: None,
            prefer_usb: true,
            auto_use_usb: true,
            leased: vec![],
        }
    }
    #[test]
    fn automatic_joint_allocation_preserves_the_only_awdl_radio() {
        let inv = inventory(vec![radio("a-dual", true), radio("b-direct", false)]);
        let mut req = request();
        req.preferred = Some("a-dual".into());
        let plan = allocate(&inv, &req);
        assert_eq!(plan.airdrop.unwrap().radio_id, "a-dual");
        assert_eq!(plan.quickshare.as_deref(), Some("b-direct"));
        let one = inventory(vec![radio("only", true)]);
        assert!(allocate(&one, &request()).quickshare.is_none());
        req.airdrop = false;
        req.preferred = None;
        assert_eq!(allocate(&one, &req).quickshare.as_deref(), Some("only"));
    }
    #[test]
    fn constraints_override_preferences_and_usb_opt_out_is_respected() {
        let mut inv = inventory(vec![
            radio("blocked", true),
            radio("busy", true),
            radio("leased", true),
            radio("manual", true),
        ]);
        inv.radios[0].rfkill = true;
        inv.interfaces[1].nm_state = Some(50); // Connecting, even before carrier.
        let mut req = request();
        req.auto_use_usb = false;
        req.preferred = Some("busy".into());
        req.leased = vec!["phy-leased".into()];
        let plan = allocate(&inv, &req);
        assert!(plan.airdrop.is_none() && plan.quickshare.is_none());
        req.preferred = Some("manual".into());
        assert_eq!(allocate(&inv, &req).airdrop.unwrap().radio_id, "manual");
        req.airdrop = false;
        assert_eq!(allocate(&inv, &req).quickshare.as_deref(), Some("manual"));
        req.leased.push("manual".into());
        assert!(allocate(&inv, &req).quickshare.is_none());
    }
    #[test]
    fn joint_choice_outweighs_greedy_preference_and_is_deterministic() {
        let mut inv = inventory(vec![radio("a-dual", true), radio("b-awdl", true)]);
        inv.radios[1].modes = vec!["monitor".into()];
        let mut req = request();
        req.preferred = Some("a-dual".into());
        let plan = allocate(&inv, &req);
        assert_eq!(plan.airdrop.unwrap().radio_id, "b-awdl");
        assert_eq!(plan.quickshare.as_deref(), Some("a-dual"));
        inv.radios.reverse();
        assert_eq!(allocate(&inv, &req).quickshare, plan.quickshare);
        inv.radios[0].channels[0].no_ir = true;
        let plan = allocate(&inv, &req);
        assert_eq!(plan.airdrop.unwrap().radio_id, "a-dual");
        assert!(plan.quickshare.is_none());
    }
    #[test]
    fn aliases_on_same_phy_never_count_as_two_radios() {
        let mut inv = inventory(vec![radio("a", true), radio("b", true)]);
        inv.radios[1].phy = inv.radios[0].phy.clone();
        let plan = allocate(&inv, &request());
        assert!(plan.airdrop.is_some() && plan.quickshare.is_none());
        let mut req = request();
        req.leased = vec![inv.radios[0].phy.clone()];
        let plan = allocate(&inv, &req);
        assert!(plan.airdrop.is_none() && plan.quickshare.is_none());
    }
    #[test]
    fn hotplug_waits_for_idle_then_queues_once_without_stealing_owned_radios() {
        let mut watch = HotplugAllocation::default();
        let mut req = request();
        req.airdrop = false;
        let empty = Inventory::default();
        assert!(!watch.observe(&empty, &req, false));
        let inv = inventory(vec![radio("usb", true)]);
        assert!(!watch.observe(&inv, &req, true)); // Active transfer at arrival.
        assert!(watch.observe(&inv, &req, false)); // No further hotplug needed.
        watch.queued();
        assert!(!watch.observe(&inv, &req, false)); // No repeated restart.
        let mut transient = inv.clone();
        transient.radios[0].interfaces.push("owned-monitor".into());
        assert!(!watch.observe(&transient, &req, false));
        assert!(!watch.observe(&inv, &req, false)); // Failed-engine VIF churn cannot loop.
        assert!(!watch.observe(&empty, &req, false));
        req.leased.push("phy-usb".into());
        assert!(!watch.observe(&inv, &req, false)); // Other backend owns it.
        req.leased.clear();
        let mut blocked = inv.clone();
        blocked.radios[0].rfkill = true;
        assert!(!watch.observe(&blocked, &req, false));
        assert!(watch.observe(&inv, &req, false)); // Unblocked suitable adapter.
    }
}
