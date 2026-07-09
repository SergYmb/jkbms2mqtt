use crate::mqtt::inbound::{IncomingRequest, parse_inbound_command};

#[test]
fn parse_charging_on() {
    assert_eq!(
        parse_inbound_command("my_jk_bms/charging/set", b"ON"),
        Some(IncomingRequest::SetCharging(true))
    );
}

#[test]
fn parse_charging_off() {
    assert_eq!(
        parse_inbound_command("my_jk_bms/charging/set", b"OFF"),
        Some(IncomingRequest::SetCharging(false))
    );
}

#[test]
fn parse_balancing_on() {
    assert_eq!(
        parse_inbound_command("my_jk_bms/balancing/set", b"ON"),
        Some(IncomingRequest::SetBalancing(true))
    );
}

#[test]
fn parse_balancing_off() {
    assert_eq!(
        parse_inbound_command("my_jk_bms/balancing/set", b"OFF"),
        Some(IncomingRequest::SetBalancing(false))
    );
}

#[test]
fn parse_case_insensitive() {
    for payload in [b"on".as_slice(), b"On", b"oN", b"ON"] {
        assert_eq!(
            parse_inbound_command("my_jk_bms/charging/set", payload),
            Some(IncomingRequest::SetCharging(true))
        );
    }
    assert_eq!(
        parse_inbound_command("my_jk_bms/charging/set", b"garbage"),
        None
    );
}

#[test]
fn parse_unknown_topic_returns_none() {
    assert_eq!(parse_inbound_command("some/other/topic", b"whatever"), None);
    assert_eq!(
        parse_inbound_command("my_jk_bms/total_voltage/state", b"27.36"),
        None
    );
}
