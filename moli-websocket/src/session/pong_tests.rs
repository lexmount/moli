use super::{Assembler, Closing, Flight, Session};
use crate::{
    Command, Event,
    commands::command_channel,
    frames::{Received, close_payload},
};
use moli_curl::websocket::{MAX_SEND_FRAME_BYTES, WsFlags};

fn session() -> Session {
    Session {
        socket_id: 84,
        outbox: Default::default(),
        outgoing: Default::default(),
        pending_pong: Default::default(),
        assembler: Assembler::default(),
        closing: Closing::default(),
        terminal: false,
    }
}

#[tokio::test]
async fn pending_pongs_coalesce_while_a_data_frame_is_in_flight() {
    let mut session = session();
    let (port, mut commands) = command_channel();
    port.send(Command::SendBinary(vec![7; 2 * MAX_SEND_FRAME_BYTES]))
        .unwrap();
    session.command(commands.recv().await.unwrap());
    let (first, flight) = session.next_frame().unwrap();
    // The driver cannot submit another frame before this completion. Process
    // incoming Pings with that completion deliberately withheld, without sleeps.
    for payload in 0..10 {
        session.received(Received::Ping(vec![payload]));
        assert!(
            !session.terminal,
            "Ping backlog must coalesce under send backpressure"
        );
    }
    session.sent(flight);
    let (pong, flight) = session.next_frame().unwrap();
    assert_eq!(pong.flags, WsFlags::PONG);
    assert_eq!(pong.data, [9]);
    session.sent(flight);
    let (last, flight) = session.next_frame().unwrap();
    assert_eq!(last.flags, WsFlags::BINARY);
    assert_eq!(
        [first.data, last.data].concat(),
        vec![7; 2 * MAX_SEND_FRAME_BYTES]
    );
    session.sent(flight);
    assert!(
        matches!(session.outbox.pop_front(), Some(Event::SendCompleted { payload_length, .. }) if payload_length == 2 * MAX_SEND_FRAME_BYTES)
    );
    assert!(session.next_frame().is_none());

    let payload = close_payload(Some(1000), String::new()).unwrap();
    session.begin_close(payload.clone(), true);
    let (close, flight) = session.next_frame().unwrap();
    assert_eq!(close.flags, WsFlags::CLOSE);
    session.sent(flight);
    session.received(Received::Close {
        code: 1000,
        reason: String::new(),
        payload,
    });
    assert!(session.terminal);
    assert!(matches!(
        session.outbox.pop_front(),
        Some(Event::Closing { .. })
    ));
    assert!(matches!(
        session.outbox.pop_front(),
        Some(Event::Close {
            code: 1000,
            was_clean: true,
            ..
        })
    ));
    assert!(session.outbox.is_empty());
}

#[test]
fn in_flight_pong_retains_its_payload_while_later_pings_coalesce() {
    let mut session = session();
    session.received(Received::Ping(vec![42]));
    let (in_flight, flight) = session.next_frame().unwrap();
    assert!(matches!(flight, Flight::Pong));
    for payload in 0..10 {
        session.received(Received::Ping(vec![payload]));
        assert!(!session.terminal);
    }
    // An empty Ping still requires an empty Pong; it is not "no pending Pong".
    session.received(Received::Ping(Vec::new()));
    assert_eq!(in_flight.data, [42]);
    session.sent(flight);
    let (latest, flight) = session.next_frame().unwrap();
    assert_eq!(latest.flags, WsFlags::PONG);
    assert!(latest.data.is_empty());
    session.sent(flight);
    assert!(session.next_frame().is_none());
}

#[test]
fn peer_close_discards_unsent_pong() {
    let mut session = session();
    session.received(Received::Ping(vec![1]));
    session.received(Received::Close {
        code: 1000,
        reason: String::new(),
        payload: close_payload(Some(1000), String::new()).unwrap(),
    });
    let (reply, flight) = session.next_frame().unwrap();
    assert_eq!(reply.flags, WsFlags::CLOSE);
    session.sent(flight);
    assert!(session.terminal);
    assert!(session.next_frame().is_none());
}
