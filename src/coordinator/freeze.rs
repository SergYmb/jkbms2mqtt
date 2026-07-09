use crate::domain::Snapshot;

#[derive(Debug, Clone, Copy)]
pub(crate) enum SwitchField {
    Charging,
    Balancing,
}

pub(crate) struct SwitchFreeze {
    charging: Option<(u64, bool)>,
    balancing: Option<(u64, bool)>,
}

impl SwitchFreeze {
    pub(crate) fn new() -> Self {
        Self {
            charging: None,
            balancing: None,
        }
    }

    pub(crate) fn freeze(&mut self, seq: u64, switch: SwitchField, value: bool) {
        match switch {
            SwitchField::Charging => self.charging = Some((seq, value)),
            SwitchField::Balancing => self.balancing = Some((seq, value)),
        }
    }

    /// Override the snapshot's switch fields with the optimistic values.
    pub(crate) fn apply_to(&self, snapshot: &mut Snapshot) {
        if let Some((_, value)) = self.charging {
            snapshot.charging_switch = value;
        }
        if let Some((_, value)) = self.balancing {
            snapshot.balance_switch = value;
        }
    }

    /// Called on write confirmation: clear the freeze for the given seq (or any older seq).
    /// Newer-or-equal confirmation clears the freeze; stale confirmation (seq < frozen_seq) is ignored.
    pub(crate) fn apply(&mut self, seq: u64) {
        if let Some((frozen_seq, _)) = self.charging {
            if seq >= frozen_seq {
                self.charging = None;
            }
        }
        if let Some((frozen_seq, _)) = self.balancing {
            if seq >= frozen_seq {
                self.balancing = None;
            }
        }
    }

    /// Called on write error: clear the freeze only for the exact seq that failed.
    pub(crate) fn clear(&mut self, seq: u64) {
        if let Some((frozen_seq, _)) = self.charging {
            if seq == frozen_seq {
                self.charging = None;
            }
        }
        if let Some((frozen_seq, _)) = self.balancing {
            if seq == frozen_seq {
                self.balancing = None;
            }
        }
    }
}
