use crate::{
    node_mock::NodeMock,
    test_suites::two_basic::{test_author_one_aspect, test_send_message, TwoNodesTestFn},
};
use lib3h_protocol::protocol_server::Lib3hServerProtocol;

lazy_static! {
    pub static ref TWO_NODES_CONNECTION_TEST_FNS: Vec<(TwoNodesTestFn, bool)> = vec![
        (test_two_disconnect, true),
        (test_two_gossip_self, true),
        (test_two_peer_timeout, true),
        (test_two_peer_timeout_reconnect, true),
        (test_two_reconnect, true),
    ];
}

//--------------------------------------------------------------------------------------------------
// Tests
//--------------------------------------------------------------------------------------------------

/// Have Alex disconnect and reconnect
fn test_two_disconnect(alex: &mut NodeMock, billy: &mut NodeMock) {
    alex.disconnect();
    let (did_work, srv_msg_list) = alex.process().unwrap();
    assert_eq!(srv_msg_list.len(), 0);
    println!("srv_msg_list = {:?} ({})\n", srv_msg_list, did_work);
    // Should be disconnected from the network
    let (did_work, srv_msg_list) = billy.process().unwrap();
    println!("srv_msg_list = {:?} ({})\n", srv_msg_list, did_work);
    assert!(did_work);
    assert_eq!(srv_msg_list.len(), 1);
    let msg_1 = &srv_msg_list[0];
    one_let!(Lib3hServerProtocol::Disconnected(response) = msg_1 {
        assert_eq!(response.network_id, "FIXME");
    });
}

/// Wait for peer timeout
fn test_two_gossip_self(alex: &mut NodeMock, billy: &mut NodeMock) {
    // Wait before peer Timeout threshold
    std::thread::sleep(std::time::Duration::from_millis(1000));
    // Billy should send a PeerTimedOut message
    let (did_work, srv_msg_list) = billy.process().unwrap();
    println!("srv_msg_list = {:?} ({})\n", srv_msg_list, did_work);
    assert!(did_work);
    assert_eq!(srv_msg_list.len(), 0);

    // Alex should gossip self
    let (did_work, srv_msg_list) = alex.process().unwrap();
    println!("srv_msg_list = {:?} ({})\n", srv_msg_list, did_work);
    assert!(did_work);
    assert_eq!(srv_msg_list.len(), 0);

    // More process: Have Billy process P2p::PeerAddress of alex
    let (_did_work, _srv_msg_list) = billy.process().unwrap();
    let (_did_work, _srv_msg_list) = alex.process().unwrap();

    // Wait past peer Timeout threshold
    std::thread::sleep(std::time::Duration::from_millis(2100));
    // Billy should not see a PeerTimedOut message
    let (did_work, srv_msg_list) = billy.process().unwrap();
    println!("srv_msg_list = {:?} ({})\n", srv_msg_list, did_work);
    assert!(did_work);
    assert_eq!(srv_msg_list.len(), 0);
}

/// Wait for peer timeout
fn test_two_peer_timeout(_alex: &mut NodeMock, billy: &mut NodeMock) {
    // Wait before peer Timeout threshold
    std::thread::sleep(std::time::Duration::from_millis(1000));
    // Billy should NOT send a PeerTimedOut message
    let (did_work, srv_msg_list) = billy.process().unwrap();
    assert!(did_work);
    assert_eq!(srv_msg_list.len(), 0);
    // Wait past peer Timeout threshold
    std::thread::sleep(std::time::Duration::from_millis(2100));
    // Billy SHOULD send a PeerTimedOut message ...
    let (did_work, srv_msg_list) = billy.process().unwrap();
    println!("srv_msg_list = {:?} ({})\n", srv_msg_list, did_work);
    assert!(did_work);
    // ... resulting in a Disconnected on next process loop
    let (did_work, srv_msg_list) = billy.process().unwrap();
    println!("srv_msg_list = {:?} ({})\n", srv_msg_list, did_work);
    assert!(did_work);
    assert_eq!(srv_msg_list.len(), 1);
    let msg_1 = &srv_msg_list[0];
    one_let!(Lib3hServerProtocol::Disconnected(response) = msg_1 {
        assert_eq!(response.network_id, "FIXME");
    });
}

/// Wait for peer timeout than reconnect
fn test_two_peer_timeout_reconnect(alex: &mut NodeMock, billy: &mut NodeMock) {
    // Wait past peer Timeout threshold
    std::thread::sleep(std::time::Duration::from_millis(3100));
    // Billy SHOULD send a PeerTimedOut message ...
    let (did_work, srv_msg_list) = billy.process().unwrap();
    println!("srv_msg_list = {:?} ({})\n", srv_msg_list, did_work);
    assert!(did_work);
    // ... resulting in a Disconnected on next process loop
    let (did_work, srv_msg_list) = billy.process().unwrap();
    println!("srv_msg_list = {:?} ({})\n", srv_msg_list, did_work);
    assert!(did_work);
    assert_eq!(srv_msg_list.len(), 1);
    let msg_1 = &srv_msg_list[0];
    one_let!(Lib3hServerProtocol::Disconnected(response) = msg_1 {
        assert_eq!(response.network_id, "FIXME");
    });

    let (did_work, srv_msg_list) = billy.process().unwrap();
    println!("srv_msg_list = {:?} ({})\n", srv_msg_list, did_work);
    assert!(!did_work);
    assert_eq!(srv_msg_list.len(), 0);

    // Alex shows signs of life, but should receive a disconnect from billy
    println!("\nAlex gossips self \n");
    let (did_work, srv_msg_list) = alex.process().unwrap();
    println!("srv_msg_list = {:?} ({})\n", srv_msg_list, did_work);
    assert!(did_work);
    let msg_1 = &srv_msg_list[0];
    one_let!(Lib3hServerProtocol::Disconnected(response) = msg_1 {
        assert_eq!(response.network_id, "FIXME");
    });
    // So alex reconnects
    println!("\n Reconnecting Alex...\n");
    alex.reconnect().expect("Reconnection failed");
    let (did_work, srv_msg_list) = alex.process().unwrap();
    println!("srv_msg_list = {:?} ({})\n", srv_msg_list, did_work);
    assert!(did_work);

    // Billy should see a reconnect
    let (did_work, srv_msg_list) = billy.process().unwrap();
    println!("srv_msg_list = {:?} ({})\n", srv_msg_list, did_work);
    assert!(did_work);
    assert_eq!(srv_msg_list.len(), 1);
    let msg_1 = &srv_msg_list[0];
    one_let!(Lib3hServerProtocol::Connected(response) = msg_1 {
        assert_eq!(response.uri, alex.advertise());
    });

    // More process
    let (_did_work, _srv_msg_list) = alex.process().unwrap();
    let (_did_work, _srv_msg_list) = billy.process().unwrap();
    let (_did_work, _srv_msg_list) = alex.process().unwrap();

    test_send_message(alex, billy);
    test_author_one_aspect(alex, billy);
}

/// Have Alex disconnect and reconnect
fn test_two_reconnect(alex: &mut NodeMock, billy: &mut NodeMock) {
    alex.disconnect();
    let (did_work, srv_msg_list) = alex.process().unwrap();
    assert_eq!(srv_msg_list.len(), 0);
    println!(
        "disconnect srv_msg_list = {:?} ({})\n",
        srv_msg_list, did_work
    );
    // Should be disconnected from the network
    let (did_work, srv_msg_list) = billy.process().unwrap();
    println!("srv_msg_list = {:?} ({})\n", srv_msg_list, did_work);
    assert!(did_work);
    assert_eq!(srv_msg_list.len(), 1);
    let msg_1 = &srv_msg_list[0];
    one_let!(Lib3hServerProtocol::Disconnected(response) = msg_1 {
        assert_eq!(response.network_id, "FIXME");
    });

    println!("\n Reconnecting Alex...\n");
    alex.reconnect().expect("Reconnection failed");

    let (did_work, srv_msg_list) = alex.process().unwrap();
    println!(
        "reconnect srv_msg_list = {:?} ({})\n",
        srv_msg_list, did_work
    );
    assert!(did_work);
    assert_eq!(srv_msg_list.len(), 4);

    let (did_work, srv_msg_list) = billy.process().unwrap();
    println!(
        "reconnect srv_msg_list = {:?} ({})\n",
        srv_msg_list, did_work
    );
    assert!(did_work);
    assert_eq!(srv_msg_list.len(), 1);
    let msg_1 = &srv_msg_list[0];
    one_let!(Lib3hServerProtocol::Connected(response) = msg_1 {
        assert_eq!(response.uri, alex.advertise());
    });

    // More process
    let (_did_work, _srv_msg_list) = alex.process().unwrap();
    let (_did_work, _srv_msg_list) = billy.process().unwrap();
    let (_did_work, _srv_msg_list) = alex.process().unwrap();

    test_send_message(alex, billy);
    test_author_one_aspect(alex, billy);
}