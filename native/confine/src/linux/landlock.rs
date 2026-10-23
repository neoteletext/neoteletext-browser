//! Landlock: the paths a role reads and writes, and its TCP, enforced
//! whole or not at all (decision 58).

use landlock::{
    ABI, Access, AccessFs, AccessNet, CompatLevel, Compatible, NetPort, PathBeneath, PathFd, Ruleset, RulesetAttr, RulesetCreatedAttr, RulesetStatus,
};
use std::path::Path;

use super::paths::{SOUND_DEVICES, readable};
use crate::Role;

/// The Landlock the rules are written for (kernel 6.10 and later); a
/// kernel that cannot enforce all of it is refused (decision 58).
pub(super) const LANDLOCK_ABI: ABI = ABI::V5;

/// Read access to `path`: a file cannot be given a directory's rights, so
/// a file is given only the read rights a file has. Asked for on a file,
/// a directory's rights would leave the ruleset enforced only in part.
fn read_access(path: &Path, abi: ABI) -> landlock::BitFlags<AccessFs> {
    if path.is_dir() { AccessFs::from_read(abi) } else { AccessFs::from_read(abi) & AccessFs::from_file(abi) }
}

/// Landlock: the readable paths, the sound devices for the sound helper,
/// the core's settings folder or the updater's staging folder written, no
/// TCP for a role without the network and one port for the updater, every
/// one of them enforced or none (decision 58).
pub(super) fn landlock(role: Role, writable: Option<&Path>) -> Result<(), String> {
    let abi = LANDLOCK_ABI;
    let mut ruleset =
        Ruleset::default().set_compatibility(CompatLevel::HardRequirement).handle_access(AccessFs::from_all(abi)).map_err(|error| error.to_string())?;
    if role != Role::Network {
        ruleset = ruleset.handle_access(AccessNet::from_all(abi)).map_err(|error| error.to_string())?;
    }
    let mut created = ruleset.create().map_err(|error| error.to_string())?;
    for path in readable(role) {
        let Ok(fd) = PathFd::new(&path) else { continue };
        created = created.add_rule(PathBeneath::new(fd, read_access(&path, abi))).map_err(|error| error.to_string())?;
    }
    let written = match role {
        Role::Sound => Some(Path::new(SOUND_DEVICES)),
        Role::Core | Role::Updater { .. } => writable,
        Role::Decoder | Role::Network | Role::Interface => None,
    };
    if let Some(folder) = written
        && let Ok(fd) = PathFd::new(folder)
    {
        created = created.add_rule(PathBeneath::new(fd, AccessFs::from_all(abi))).map_err(|error| error.to_string())?;
    }
    // The updater connects to its one port, and binds none (section 8.1
    // of the installation design).
    if let Role::Updater { port } = role {
        created = created.add_rule(NetPort::new(port, AccessNet::ConnectTcp)).map_err(|error| error.to_string())?;
    }
    let status = created.restrict_self().map_err(|error| error.to_string())?;
    enforced(&status.ruleset)
}

/// Only a ruleset the kernel enforces whole is a sandbox (decision 58).
fn enforced(status: &RulesetStatus) -> Result<(), String> {
    match status {
        RulesetStatus::FullyEnforced => Ok(()),
        RulesetStatus::PartiallyEnforced => Err(String::from("this kernel enforces only part of the rules (Landlock 5, Linux 6.10, is needed)")),
        RulesetStatus::NotEnforced => Err(String::from("this kernel has no Landlock (Landlock 5, Linux 6.10, is needed)")),
    }
}
