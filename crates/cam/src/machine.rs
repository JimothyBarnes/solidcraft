//! The machines a setup can be on (decision 0036 of the CAM work), and what the CAM engine knows
//! of one so far: its name and its top spindle speed, which caps the speed an operation works
//! out from its tool's cutting data. Built in for now; machine files a shop writes for its own
//! machines (with their travels, their posts and, for P4, their kinematics) come later.

use crate::feeds::GENERIC_MAX_RPM;

/// A machine a setup can be on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Machine {
    /// What a setup names it by (`Setup::machine`).
    pub id: &'static str,
    /// What it is called, as the app shows it: empty for the generic mill, whose name is the
    /// app's own words.
    pub name: &'static str,
    /// Its top spindle speed, rev/min.
    pub max_rpm: f64,
}

/// The id of the machine a setup is on when it names none.
pub const GENERIC_MILL: &str = "generic_mill";

/// The machines a setup can name: a generic mill (any machine, its speeds kept to what most
/// can run), then the machines the work is tested on. The owner's Haas VF-4SS has the 12 000
/// rev/min spindle of the SS models (the owner, 2026-11-24).
pub const BUILTIN_MACHINES: [Machine; 2] =
    [Machine { id: GENERIC_MILL, name: "", max_rpm: GENERIC_MAX_RPM }, Machine { id: "haas_vf4ss", name: "Haas VF-4SS", max_rpm: 12_000.0 }];

/// The machine a setup naming `id` is on: the generic mill when it names none; `None` for an id
/// this version does not know (a design from a later one, a script's typing slip).
pub fn machine(id: Option<&str>) -> Option<&'static Machine> {
    let id = id.unwrap_or(GENERIC_MILL);
    BUILTIN_MACHINES.iter().find(|machine| machine.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_setup_is_on_the_generic_mill_unless_it_names_a_machine_this_version_knows() {
        assert_eq!(machine(None).map(|m| (m.id, m.max_rpm)), Some((GENERIC_MILL, 10_000.0)));
        assert_eq!(machine(Some("haas_vf4ss")).map(|m| (m.name, m.max_rpm)), Some(("Haas VF-4SS", 12_000.0)));
        assert_eq!(machine(Some("generic_mill")), machine(None));
        for unknown in ["", "haas", "HAAS_VF4SS", "a".repeat(10_000).as_str()] {
            assert!(machine(Some(unknown)).is_none(), "{unknown}");
        }
    }
}
