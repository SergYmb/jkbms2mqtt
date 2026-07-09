#[derive(Debug, PartialEq)]
pub enum IncomingRequest {
    SetCharging(bool),
    SetBalancing(bool),
}

pub(super) fn inbound_commands() -> &'static [&'static str] {
    &["charging", "balancing"]
}

pub fn parse_inbound_command(topic: &str, payload: &[u8]) -> Option<IncomingRequest> {
    if topic.ends_with("/charging/set") {
        parse_on_off(payload).map(IncomingRequest::SetCharging)
    } else if topic.ends_with("/balancing/set") {
        parse_on_off(payload).map(IncomingRequest::SetBalancing)
    } else {
        None
    }
}

fn parse_on_off(payload: &[u8]) -> Option<bool> {
    match payload.to_ascii_uppercase().as_slice() {
        b"ON" => Some(true),
        b"OFF" => Some(false),
        _ => None,
    }
}
