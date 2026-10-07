//! Recovery may cancel a recorded pending operation, never an unknown group.
use super::*;
use std::collections::BTreeMap;

fn interfaces_at(sys: &Path, phy: &str) -> Result<BTreeMap<String, u32>, String> {
    let radio = std::fs::canonicalize(sys.join("class/ieee80211").join(phy))
        .map_err(|error| error.to_string())?;
    let mut result = BTreeMap::new();
    for entry in std::fs::read_dir(sys.join("class/net")).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let link = entry.path().join("phy80211");
        if !link.try_exists().map_err(|error| error.to_string())? {
            continue;
        }
        if std::fs::canonicalize(link).map_err(|error| error.to_string())? != radio {
            continue;
        }
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| "Invalid interface name")?;
        let index = std::fs::read_to_string(entry.path().join("ifindex"))
            .map_err(|error| error.to_string())?
            .trim()
            .parse::<u32>()
            .map_err(|error| error.to_string())?;
        result.insert(name, index);
    }
    Ok(result)
}

pub(super) async fn prepare(lease: &Lease) -> Result<linuxdrop_netd::P2pRecovery, String> {
    let kernel_interfaces = interfaces_at(Path::new("/sys"), &lease.phy)?;
    if !kernel_interfaces.contains_key(&lease.interface) {
        return Err("Reserved interface disappeared before P2P preparation".into());
    }
    let formation = linuxdrop_network::p2p::prepare_formation(&lease.interface)
        .await
        .map_err(|error| error.to_string())?;
    if interfaces_at(Path::new("/sys"), &lease.phy)? != kernel_interfaces {
        return Err("Radio interfaces changed during P2P preparation".into());
    }
    Ok(linuxdrop_netd::P2pRecovery {
        formation,
        kernel_interfaces,
    })
}

pub(super) async fn recover(lease: &Lease) -> Result<(), String> {
    let saved = lease.p2p_recovery.as_ref().ok_or(P2P_UNCERTAIN)?;
    if saved.formation.parent_interface != lease.interface
        || !saved.kernel_interfaces.contains_key(&lease.interface)
    {
        return Err("Formation provenance does not belong to the reserved interface".into());
    }
    // Kernel wiphy removal retires the original radio and its operations. It
    // requires no mutation of a subsequently inserted adapter.
    if !Path::new("/sys/class/ieee80211")
        .join(&lease.phy)
        .try_exists()
        .map_err(|e| e.to_string())?
    {
        return Ok(());
    }
    let verify = || -> Result<(), String> {
        if interfaces_at(Path::new("/sys"), &lease.phy)? != saved.kernel_interfaces {
            return Err("Radio interfaces changed; pending group ownership is uncertain".into());
        }
        Ok(())
    };
    verify()?;
    linuxdrop_network::p2p::recover_formation(&saved.formation)
        .await
        .map_err(|error| error.to_string())?;
    verify()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn kernel_provenance_detects_recreated_and_added_interfaces() {
        let root =
            std::env::temp_dir().join(format!("linuxdrop-p2p-provenance-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("class/ieee80211/phy-test")).unwrap();
        let add = |name: &str, index: u32| {
            let interface = root.join("class/net").join(name);
            std::fs::create_dir_all(&interface).unwrap();
            std::os::unix::fs::symlink(
                root.join("class/ieee80211/phy-test"),
                interface.join("phy80211"),
            )
            .unwrap();
            std::fs::write(interface.join("ifindex"), index.to_string()).unwrap();
        };
        add("reserved0", 42);
        let original = interfaces_at(&root, "phy-test").unwrap();
        assert_eq!(original["reserved0"], 42);
        std::fs::write(root.join("class/net/reserved0/ifindex"), "43").unwrap();
        assert_ne!(interfaces_at(&root, "phy-test").unwrap(), original);
        std::fs::write(root.join("class/net/reserved0/ifindex"), "42").unwrap();
        add("unidentified0", 44);
        assert_ne!(interfaces_at(&root, "phy-test").unwrap(), original);
        std::fs::write(root.join("class/net/unidentified0/ifindex"), "invalid").unwrap();
        assert!(interfaces_at(&root, "phy-test").is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
