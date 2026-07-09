use crate::coordinator::freeze::{SwitchField, SwitchFreeze};
use crate::domain::Snapshot;

fn snap_with(charging: bool, balancing: bool) -> Snapshot {
    Snapshot {
        charging_switch: charging,
        balance_switch: balancing,
        ..Default::default()
    }
}

#[test]
fn freeze_then_apply_to_overrides_snapshot() {
    let mut freeze = SwitchFreeze::new();
    freeze.freeze(1, SwitchField::Charging, true);

    let mut snap = snap_with(false, false);
    freeze.apply_to(&mut snap);

    assert!(
        snap.charging_switch,
        "freeze should override charging to true"
    );
    assert!(
        !snap.balance_switch,
        "balancing not frozen, should be unchanged"
    );
}

#[test]
fn apply_clears_on_matching_seq() {
    let mut freeze = SwitchFreeze::new();
    freeze.freeze(1, SwitchField::Charging, true);
    freeze.apply(1);

    let mut snap = snap_with(false, false);
    freeze.apply_to(&mut snap);
    assert!(
        !snap.charging_switch,
        "freeze should be cleared after apply(seq=1)"
    );
}

#[test]
fn apply_clears_on_newer_seq() {
    let mut freeze = SwitchFreeze::new();
    freeze.freeze(1, SwitchField::Charging, true);
    freeze.apply(2); // newer seq

    let mut snap = snap_with(false, false);
    freeze.apply_to(&mut snap);
    assert!(!snap.charging_switch, "freeze cleared by newer seq");
}

#[test]
fn apply_ignores_stale_seq() {
    let mut freeze = SwitchFreeze::new();
    freeze.freeze(2, SwitchField::Charging, true);
    freeze.apply(1); // stale — must NOT clear freeze

    let mut snap = snap_with(false, false);
    freeze.apply_to(&mut snap);
    assert!(snap.charging_switch, "freeze must survive stale apply");
}

#[test]
fn clear_on_error_unfreezes_matching_seq() {
    let mut freeze = SwitchFreeze::new();
    freeze.freeze(1, SwitchField::Charging, true);
    freeze.clear(1);

    let mut snap = snap_with(false, false);
    freeze.apply_to(&mut snap);
    assert!(
        !snap.charging_switch,
        "freeze cleared by error on matching seq"
    );
}

#[test]
fn clear_ignores_different_seq() {
    let mut freeze = SwitchFreeze::new();
    freeze.freeze(2, SwitchField::Charging, true);
    freeze.clear(1); // different seq — must NOT clear

    let mut snap = snap_with(false, false);
    freeze.apply_to(&mut snap);
    assert!(
        snap.charging_switch,
        "freeze must survive clear with mismatched seq"
    );
}

#[test]
fn both_switches_independent() {
    let mut freeze = SwitchFreeze::new();
    freeze.freeze(1, SwitchField::Charging, true);
    freeze.freeze(2, SwitchField::Balancing, true);

    // Clear charging error (seq 1), balancing still frozen
    freeze.clear(1);

    let mut snap = snap_with(false, false);
    freeze.apply_to(&mut snap);
    assert!(!snap.charging_switch, "charging freeze cleared");
    assert!(snap.balance_switch, "balancing freeze still active");
}
