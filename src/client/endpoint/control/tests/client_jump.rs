//! A jump the daemon forwards decodes from its endpoint control.

use super::*;

#[test]
fn a_forwarded_jump_decodes_from_the_daemons_control() {
    let jump = crate::protocol::endpoint::EndpointClientJump {
        jump_id: "jump-3".into(),
        endpoint_id: "ssh:0123456789abcdef0123456789abcdef".into(),
        boot_id: "boot-b".into(),
        pane_id: "w1:p4".into(),
    };
    let crate::protocol::ServerMessage::EndpointControl { kind, data } =
        crate::protocol::endpoint::client_jump_message(&jump).unwrap()
    else {
        panic!("expected a control");
    };

    let EndpointControlMessage::Contract(crate::client::client_contract::Control::ClientJump(
        decoded,
    )) = decode_endpoint_control(&kind, &data).unwrap()
    else {
        panic!("expected a jump");
    };

    assert_eq!(decoded, jump);
    // The contract fixture is the same payload.
    let EndpointControlMessage::Contract(crate::client::client_contract::Control::ClientJump(
        fixture,
    )) = decode_endpoint_control(
        &kind,
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/endpoint-client-jump-v1.json"
        )),
    )
    .unwrap()
    else {
        panic!("expected a jump");
    };
    assert_eq!(fixture, jump);
    // A jump this client cannot read has no id to answer under.
    assert!(matches!(
        decode_endpoint_control(&kind, "{}").unwrap(),
        EndpointControlMessage::Ignored
    ));
}
