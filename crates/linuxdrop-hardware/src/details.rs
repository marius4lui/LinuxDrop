use serde::{Deserialize, Serialize};

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DriverDetails {
    pub version: Option<String>,
    pub firmware: Option<String>,
    pub bus_info: Option<String>,
    pub source: String,
}

pub fn parse_ethtool(text: &str) -> DriverDetails {
    let field = |name: &str| {
        text.lines().find_map(|line| {
            let (key, value) = line.split_once(':')?;
            (key == name && !value.trim().is_empty()).then(|| value.trim().to_owned())
        })
    };
    DriverDetails {
        version: field("version"),
        firmware: field("firmware-version"),
        bus_info: field("bus-info"),
        source: "ethtool driver information".into(),
    }
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InterfaceLimit {
    pub modes: Vec<String>,
    pub maximum: u32,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InterfaceCombination {
    pub limits: Vec<InterfaceLimit>,
    pub total: Option<u32>,
    pub different_channels: Option<u32>,
    pub raw: String,
}

/// Keep raw evidence as well as parsed limits: unsupported future syntax must
/// never be interpreted as permission to create concurrent interfaces.
pub fn parse_combinations(lines: &[String]) -> Vec<InterfaceCombination> {
    let mut groups: Vec<String> = Vec::new();
    for line in lines {
        if line.starts_with("* ") || groups.is_empty() {
            groups.push(line.clone());
        } else {
            groups.last_mut().unwrap().push_str(&format!(" {line}"));
        }
    }
    groups
        .into_iter()
        .map(|raw| {
            let number = |s: &str| {
                s.trim_start()
                    .split(|c: char| !c.is_ascii_digit())
                    .next()
                    .and_then(|v| v.parse().ok())
            };
            let mut limits = Vec::new();
            for part in raw.split("#{").skip(1) {
                if let Some((modes, rest)) = part.split_once('}') {
                    if let Some(maximum) = rest.trim_start().strip_prefix("<=").and_then(number) {
                        limits.push(InterfaceLimit {
                            modes: modes.split(',').map(|m| m.trim().to_owned()).collect(),
                            maximum,
                        });
                    }
                }
            }
            let total = raw.split_once("total <=").and_then(|(_, s)| number(s));
            let different_channels = raw.split_once("#channels <=").and_then(|(_, s)| number(s));
            InterfaceCombination {
                limits,
                total,
                different_channels,
                raw,
            }
        })
        .collect()
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceProfile {
    pub name: String,
    pub qualification: String,
    pub limitations: Vec<String>,
}

pub fn profile(driver: Option<&str>, bands: &[String]) -> EvidenceProfile {
    let mut p = EvidenceProfile {
        name: driver.unwrap_or("unknown").into(),
        qualification: "Candidate only; local injection and Apple interoperability unverified"
            .into(),
        limitations: Vec::new(),
    };
    if bands == ["2.4 GHz"] {
        p.limitations
            .push("2.4 GHz only: AWDL social channels 44 and 149 are unavailable".into());
    }
    if driver == Some("ath9k_htc") {
        p.limitations.push("Firmware and individual USB hardware revision must be tested; driver identity is not an injection guarantee".into());
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn combinations_preserve_shared_limit_and_channel_constraint() {
        let parsed = parse_combinations(&[
            "* #{ managed, P2P-client } <= 2,".into(),
            "#{ AP } <= 1, total <= 3, #channels <= 1".into(),
        ]);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].limits[0].modes, ["managed", "P2P-client"]);
        assert_eq!(parsed[0].limits[0].maximum, 2);
        assert_eq!(parsed[0].total, Some(3));
        assert_eq!(parsed[0].different_channels, Some(1));
        assert_eq!(
            parse_ethtool("driver: ath9k_htc\nfirmware-version: 1.4\n")
                .firmware
                .as_deref(),
            Some("1.4")
        );
    }
}
