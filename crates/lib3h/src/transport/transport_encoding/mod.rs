use crate::{
    keystore::*,
    transport::{error::*, protocol::*},
};
use detach::prelude::*;
use lib3h_crypto_api::CryptoSystem;
use lib3h_ghost_actor::prelude::*;
use std::collections::HashMap;
use url::Url;

enum ToParentContext {}

enum ToInnerContext {
    AwaitBind(
        GhostMessage<RequestToChild, RequestToParent, RequestToChildResponse, TransportError>,
    ),
    AwaitSend(
        GhostMessage<RequestToChild, RequestToParent, RequestToChildResponse, TransportError>,
    ),
}

enum ToKeystoreContext {}

/// Wraps a lower-level transport in either Open or Encrypted communication
/// Also adds a concept of MachineId and AgentId
/// This is currently a stub, only the Id concept is in place.
pub struct TransportEncoding {
    #[allow(dead_code)]
    crypto: Box<dyn CryptoSystem>,
    // the machine_id or agent_id of this encoding instance
    this_id: String,
    // the keystore to use for getting signatures for `this_id`
    keystore: Detach<KeystoreActorParentWrapperDyn<ToKeystoreContext>>,
    // our parent channel endpoint
    endpoint_parent: Option<TransportActorParentEndpoint>,
    // our self channel endpoint
    endpoint_self: Detach<
        GhostContextEndpoint<
            Self,
            ToParentContext,
            RequestToParent,
            RequestToParentResponse,
            RequestToChild,
            RequestToChildResponse,
            TransportError,
        >,
    >,
    // ref to our inner transport
    inner_transport: Detach<TransportActorParentWrapperDyn<TransportEncoding, ToInnerContext>>,
    // if we have never sent a message to this node before,
    // we need to first handshake. Store the send payload && msg object
    // we will continue the transaction once the handshake completes
    #[allow(clippy::complexity)]
    pending_send_data: HashMap<
        Url,
        Vec<(
            Vec<u8>,
            GhostMessage<RequestToChild, RequestToParent, RequestToChildResponse, TransportError>,
        )>,
    >,
    // if we have never received data from this remote before, we need to
    // handshake... this should never exactly happen... we should get a
    // handshake as the first request they send
    pending_received_data: HashMap<Url, Vec<Vec<u8>>>,
    // map low-level connection addresses to id connection addresses
    // i.e. wss://1.1.1.1:55888 -> wss://1.1.1.1:55888?a=HcMyadayada
    connections_no_id_to_id: HashMap<Url, Url>,
    // map id connection addresses to low-level connection addresses
    // i.e. wss://1.1.1.1:55888?a=HcMyadayada -> wss://1.1.1.1:55888
    connections_id_to_no_id: HashMap<Url, Url>,
}

impl TransportEncoding {
    /// create a new TransportEncoding Instance
    pub fn new(
        crypto: Box<dyn CryptoSystem>,
        this_id: String,
        keystore: DynKeystoreActor,
        inner_transport: DynTransportActor,
    ) -> Self {
        let (endpoint_parent, endpoint_self) = create_ghost_channel();
        let endpoint_parent = Some(endpoint_parent);
        let endpoint_self = Detach::new(
            endpoint_self
                .as_context_endpoint_builder()
                .request_id_prefix("enc_to_parent_")
                .build(),
        );
        let keystore = Detach::new(GhostParentWrapperDyn::new(keystore, "enc_to_keystore"));
        let inner_transport =
            Detach::new(GhostParentWrapperDyn::new(inner_transport, "enc_to_inner_"));
        Self {
            crypto,
            this_id,
            keystore,
            endpoint_parent,
            endpoint_self,
            inner_transport,
            pending_send_data: HashMap::new(),
            pending_received_data: HashMap::new(),
            connections_no_id_to_id: HashMap::new(),
            connections_id_to_no_id: HashMap::new(),
        }
    }

    /// private dispatcher for messages from our inner transport
    fn handle_msg_from_inner(
        &mut self,
        mut msg: GhostMessage<
            RequestToParent,
            RequestToChild,
            RequestToParentResponse,
            TransportError,
        >,
    ) -> TransportResult<()> {
        match msg.take_message().expect("exists") {
            RequestToParent::IncomingConnection { address } => {
                self.handle_incoming_connection(address)
            }
            RequestToParent::ReceivedData { address, payload } => {
                self.handle_received_data(address, payload)
            }
            RequestToParent::TransportError { error } => self.handle_transport_error(error),
        }
    }

    /// private send a handshake to a remote address
    fn send_handshake(&mut self, address: &Url) -> GhostResult<()> {
        self.inner_transport.publish(RequestToChild::SendMessage {
            address: address.clone(),
            payload: self.this_id.as_bytes().to_vec(),
        })
    }

    /// private handler for inner transport IncomingConnection events
    fn handle_incoming_connection(&mut self, address: Url) -> TransportResult<()> {
        match self.connections_no_id_to_id.get(&address) {
            Some(remote_addr) => {
                // if we've already seen this connection, just forward it?
                self.endpoint_self
                    .publish(RequestToParent::IncomingConnection {
                        address: remote_addr.clone(),
                    })?;
            }
            None => {
                // we've never seen this connection, handshake before
                // forwarding the IncomingConnection msg
                // (see handle_recveived_data for where it's actually sent)
                self.send_handshake(&address)?;
            }
        }
        Ok(())
    }

    /// private handler for inner transport ReceivedData events
    fn handle_received_data(&mut self, address: Url, payload: Vec<u8>) -> TransportResult<()> {
        trace!("got {:?} {}", &address, &String::from_utf8_lossy(&payload));
        match self.connections_no_id_to_id.get(&address) {
            Some(remote_addr) => {
                // if we've seen this connection before, just forward it
                self.endpoint_self.publish(RequestToParent::ReceivedData {
                    address: remote_addr.clone(),
                    payload,
                })?;
            }
            None => {
                // never seen this connection before
                // check if this is a handshake message
                // note, this is a bit of a hack right now
                // use capnproto encoding messages
                if payload.len() == 63 && payload[0] == b'H' && payload[1] == b'c' {
                    // decode the remote id
                    let remote_id = String::from_utf8_lossy(&payload);

                    // build a higher-level id address
                    let mut remote_url = address.clone();
                    remote_url.query_pairs_mut().append_pair("a", &remote_id);

                    // set up low->high and high->low mappings
                    self.connections_no_id_to_id
                        .insert(address.clone(), remote_url.clone());
                    self.connections_id_to_no_id
                        .insert(remote_url.clone(), address.clone());

                    // forward an IncomingConnection event to our parent
                    self.endpoint_self
                        .publish(RequestToParent::IncomingConnection {
                            address: remote_url.clone(),
                        })?;

                    // if we have any pending received data, send it up
                    if let Some(items) = self.pending_received_data.remove(&address) {
                        for payload in items {
                            self.endpoint_self.publish(RequestToParent::ReceivedData {
                                address: remote_url.clone(),
                                payload,
                            })?;
                        }
                    }

                    // if we have any pending send data, send it down
                    if let Some(items) = self.pending_send_data.remove(&address) {
                        for (payload, msg) in items {
                            self.fwd_send_message_result(msg, address.clone(), payload)?;
                        }
                    }
                } else {
                    // for some reason, the remote is sending us data
                    // without handshaking, let's try to handshake back?
                    self.send_handshake(&address)?;

                    // store this msg to forward after we handshake
                    let e = self
                        .pending_received_data
                        .entry(address)
                        .or_insert_with(|| vec![]);
                    e.push(payload);
                }
            }
        }
        Ok(())
    }

    /// private handler for inner transport TransportError events
    fn handle_transport_error(&mut self, error: TransportError) -> TransportResult<()> {
        // just forward this
        self.endpoint_self
            .publish(RequestToParent::TransportError { error })?;
        Ok(())
    }

    /// private dispatcher for messages coming from our parent
    fn handle_msg_from_parent(
        &mut self,
        mut msg: GhostMessage<
            RequestToChild,
            RequestToParent,
            RequestToChildResponse,
            TransportError,
        >,
    ) -> TransportResult<()> {
        match msg.take_message().expect("exists") {
            RequestToChild::Bind { spec } => self.handle_bind(msg, spec),
            RequestToChild::SendMessage { address, payload } => {
                self.handle_send_message(msg, address, payload)
            }
        }
    }

    /// private handler for Bind requests from our parent
    fn handle_bind(
        &mut self,
        msg: GhostMessage<RequestToChild, RequestToParent, RequestToChildResponse, TransportError>,
        mut spec: Url,
    ) -> TransportResult<()> {
        // remove any agent id from the spec
        // i.e. wss://1.2.3.4:55888?a=HcMyada -> wss://1.2.3.4:55888
        spec.set_query(None);

        // forward the bind to our inner_transport
        self.inner_transport.as_mut().request(
            ToInnerContext::AwaitBind(msg),
            RequestToChild::Bind { spec },
            Box::new(|m: &mut TransportEncoding, context, response| {
                let msg = {
                    match context {
                        ToInnerContext::AwaitBind(msg) => msg,
                        _ => panic!("bad context"),
                    }
                };
                let response = {
                    match response {
                        GhostCallbackData::Timeout => panic!("timeout"),
                        GhostCallbackData::Response(response) => match response {
                            Err(e) => panic!("{:?}", e),
                            Ok(response) => response,
                        },
                    }
                };
                if let RequestToChildResponse::Bind(mut data) = response {
                    // we got the bind result, append our id to it
                    // i.e. wss://1_bound?a=HcMyadyada
                    data.bound_url
                        .query_pairs_mut()
                        .append_pair("a", &m.this_id);
                    info!("got bind response: {:?}", data.bound_url);
                    msg.respond(Ok(RequestToChildResponse::Bind(data)))?;
                } else {
                    panic!("bad response to bind: {:?}", response);
                }
                Ok(())
            }),
        )?;
        Ok(())
    }

    /// handshake complete, or established connection
    /// forward SendMessage payload to our child && respond appropriately to
    /// our parent
    fn fwd_send_message_result(
        &mut self,
        msg: GhostMessage<RequestToChild, RequestToParent, RequestToChildResponse, TransportError>,
        address: Url,
        payload: Vec<u8>,
    ) -> TransportResult<()> {
        self.inner_transport.as_mut().request(
            ToInnerContext::AwaitSend(msg),
            RequestToChild::SendMessage { address, payload },
            Box::new(|_: &mut TransportEncoding, context, response| {
                let msg = {
                    match context {
                        ToInnerContext::AwaitSend(msg) => msg,
                        _ => panic!("bad context"),
                    }
                };
                let response = {
                    match response {
                        GhostCallbackData::Timeout => panic!("timeout"),
                        GhostCallbackData::Response(response) => response,
                    }
                };
                msg.respond(response)?;
                Ok(())
            }),
        )?;
        Ok(())
    }

    /// private handler for SendMessage requests from our parent
    fn handle_send_message(
        &mut self,
        msg: GhostMessage<RequestToChild, RequestToParent, RequestToChildResponse, TransportError>,
        address: Url,
        payload: Vec<u8>,
    ) -> TransportResult<()> {
        match self.connections_id_to_no_id.get(&address) {
            Some(sub_address) => {
                // we have seen this connection before
                // we can just forward the message along
                let sub_address = sub_address.clone();
                self.fwd_send_message_result(msg, sub_address, payload)?;
            }
            None => {
                // we don't have an established connection to this remote
                // we need to handshake first

                // first make a low-level address by removing the ?a=Hcyada
                let mut sub_address = address.clone();
                sub_address.set_query(None);

                // send along a handshake message
                self.send_handshake(&sub_address)?;

                // store this send_data so we can forward it after handshake
                // (see handle_received_data for where this is done)
                let e = self
                    .pending_send_data
                    .entry(sub_address)
                    .or_insert_with(|| vec![]);
                e.push((payload, msg));
            }
        }
        Ok(())
    }
}

impl
    GhostActor<
        RequestToParent,
        RequestToParentResponse,
        RequestToChild,
        RequestToChildResponse,
        TransportError,
    > for TransportEncoding
{
    fn take_parent_endpoint(&mut self) -> Option<TransportActorParentEndpoint> {
        std::mem::replace(&mut self.endpoint_parent, None)
    }

    fn process_concrete(&mut self) -> GhostResult<WorkWasDone> {
        detach_run!(&mut self.endpoint_self, |es| es.process(self))?;
        for msg in self.endpoint_self.as_mut().drain_messages() {
            self.handle_msg_from_parent(msg)?;
        }
        detach_run!(&mut self.inner_transport, |it| it.process(self))?;
        for msg in self.inner_transport.as_mut().drain_messages() {
            self.handle_msg_from_inner(msg)?;
        }
        detach_run!(&mut self.keystore, |ks| ks.process(self))?;
        Ok(false.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lib3h_sodium::SodiumCryptoSystem;

    const ID_1: &'static str = "HcSCJ9G64XDKYo433rIMm57wfI8Y59Udeb4hkVvQBZdm6bgbJ5Wgs79pBGBcuzz";
    const ID_2: &'static str = "HcMCJ8HpYvB4zqic93d3R4DjkVQ4hhbbv9UrZmWXOcn3m7w4O3AIr56JRfrt96r";

    pub struct TransportMock {
        endpoint_parent: Option<TransportActorParentEndpoint>,
        endpoint_self: Detach<
            GhostContextEndpoint<
                TransportMock,
                ToParentContext,
                RequestToParent,
                RequestToParentResponse,
                RequestToChild,
                RequestToChildResponse,
                TransportError,
            >,
        >,
        bound_url: Url,
        mock_sender: crossbeam_channel::Sender<(Url, Vec<u8>)>,
        mock_receiver: crossbeam_channel::Receiver<(Url, Vec<u8>)>,
    }

    impl TransportMock {
        pub fn new(
            mock_sender: crossbeam_channel::Sender<(Url, Vec<u8>)>,
            mock_receiver: crossbeam_channel::Receiver<(Url, Vec<u8>)>,
        ) -> Self {
            let (endpoint_parent, endpoint_self) = create_ghost_channel();
            let endpoint_parent = Some(endpoint_parent);
            let endpoint_self = Detach::new(
                endpoint_self
                    .as_context_endpoint_builder()
                    .request_id_prefix("mock_to_parent_")
                    .build(),
            );
            Self {
                endpoint_parent,
                endpoint_self,
                bound_url: Url::parse("none:").expect("can parse url"),
                mock_sender,
                mock_receiver,
            }
        }
    }

    impl
        GhostActor<
            RequestToParent,
            RequestToParentResponse,
            RequestToChild,
            RequestToChildResponse,
            TransportError,
        > for TransportMock
    {
        fn take_parent_endpoint(&mut self) -> Option<TransportActorParentEndpoint> {
            std::mem::replace(&mut self.endpoint_parent, None)
        }

        fn process_concrete(&mut self) -> GhostResult<WorkWasDone> {
            detach_run!(&mut self.endpoint_self, |es| es.process(self))?;
            for mut msg in self.endpoint_self.as_mut().drain_messages() {
                match msg.take_message().expect("exists") {
                    RequestToChild::Bind { mut spec } => {
                        spec.set_path("bound");
                        self.bound_url = spec.clone();
                        msg.respond(Ok(RequestToChildResponse::Bind(BindResultData {
                            bound_url: spec,
                        })))?;
                    }
                    RequestToChild::SendMessage { address, payload } => {
                        self.mock_sender.send((address, payload)).unwrap();
                        msg.respond(Ok(RequestToChildResponse::SendMessage))?;
                    }
                }
            }
            loop {
                match self.mock_receiver.try_recv() {
                    Ok((address, payload)) => {
                        // bit of a hack, just always send an incoming connection
                        // in front of all received data messages
                        self.endpoint_self
                            .publish(RequestToParent::IncomingConnection {
                                address: address.clone(),
                            })?;
                        self.endpoint_self
                            .publish(RequestToParent::ReceivedData { address, payload })?;
                    }
                    Err(_) => break,
                }
            }
            Ok(false.into())
        }
    }

    #[test]
    fn it_should_exchange_messages() {
        let crypto: Box<dyn CryptoSystem> =
            Box::new(SodiumCryptoSystem::new().set_pwhash_interactive());

        // set up some reference values
        let addr1 = Url::parse("test://1/bound").unwrap();
        let addr2 = Url::parse("test://2/bound").unwrap();
        let mut addr1full = addr1.clone();
        addr1full.query_pairs_mut().append_pair("a", ID_1);
        let mut addr2full = addr2.clone();
        addr2full.query_pairs_mut().append_pair("a", ID_2);

        // we need some channels into our mock inner_transports
        let (s1out, r1out) = crossbeam_channel::unbounded();
        let (s1in, r1in) = crossbeam_channel::unbounded();

        // create the first encoding transport
        let mut t1: TransportActorParentWrapper<bool, (), TransportEncoding> =
            GhostParentWrapper::new(
                TransportEncoding::new(
                    crypto.box_clone(),
                    ID_1.to_string(),
                    Box::new(KeystoreStub::new()),
                    Box::new(TransportMock::new(s1out, r1in)),
                ),
                "test1",
            );

        // give it a bind point
        t1.request(
            (),
            RequestToChild::Bind {
                spec: Url::parse("test://1").expect("can parse url"),
            },
            Box::new(|_: &mut bool, _, response| {
                assert_eq!(
                    &format!("{:?}", response),
                    "Response(Ok(Bind(BindResultData { bound_url: \"test://1/bound?a=HcSCJ9G64XDKYo433rIMm57wfI8Y59Udeb4hkVvQBZdm6bgbJ5Wgs79pBGBcuzz\" })))"
                );
                Ok(())
            })
        ).unwrap();

        // allow process
        t1.process(&mut false).unwrap();

        // we need some channels into our mock inner_transports
        let (s2out, r2out) = crossbeam_channel::unbounded();
        let (s2in, r2in) = crossbeam_channel::unbounded();

        // create the second encoding transport
        let mut t2: TransportActorParentWrapper<(), (), TransportEncoding> =
            GhostParentWrapper::new(
                TransportEncoding::new(
                    crypto.box_clone(),
                    ID_2.to_string(),
                    Box::new(KeystoreStub::new()),
                    Box::new(TransportMock::new(s2out, r2in)),
                ),
                "test2",
            );

        // give it a bind point
        t2.request(
            (),
            RequestToChild::Bind {
                spec: Url::parse("test://2").expect("can parse url"),
            },
            Box::new(|_:&mut (), _, response| {
                assert_eq!(
                    &format!("{:?}", response),
                    "Response(Ok(Bind(BindResultData { bound_url: \"test://2/bound?a=HcMCJ8HpYvB4zqic93d3R4DjkVQ4hhbbv9UrZmWXOcn3m7w4O3AIr56JRfrt96r\" })))"
                );
                Ok(())
            })
        ).unwrap();

        // allow process
        t2.process(&mut ()).unwrap();

        let mut t1_got_success_resp = false;

        // now we're going to send a message to our sibling #2
        t1.request(
            (),
            RequestToChild::SendMessage {
                address: addr2full.clone(),
                payload: b"hello".to_vec(),
            },
            Box::new(|b: &mut bool, _, response| {
                *b = true;
                // make sure we get a success response
                assert_eq!("Response(Ok(SendMessage))", format!("{:?}", response),);
                Ok(())
            }),
        )
        .unwrap();

        t1.process(&mut t1_got_success_resp).unwrap();

        // we get a handshake that needs to be forwarded to #2
        let (address, payload) = r1out.recv().unwrap();
        assert_eq!(&addr2, &address);
        assert_eq!(ID_1, &String::from_utf8_lossy(&payload));
        s2in.send((addr1.clone(), payload)).unwrap();

        t2.process(&mut ()).unwrap();
        t1.process(&mut t1_got_success_resp).unwrap();
        t2.process(&mut ()).unwrap();

        // we get a handshake that needs to be forwarded to #1
        let (address, payload) = r2out.recv().unwrap();
        assert_eq!(&addr1, &address);
        assert_eq!(ID_2, &String::from_utf8_lossy(&payload));
        s1in.send((addr2.clone(), payload)).unwrap();

        t1.process(&mut t1_got_success_resp).unwrap();
        t2.process(&mut ()).unwrap();

        // this is the process where we get the Send Success
        assert!(!t1_got_success_resp);
        t1.process(&mut t1_got_success_resp).unwrap();
        assert!(t1_got_success_resp);

        // this is another handshake due to our mock kludge
        r1out.recv().unwrap();

        t2.process(&mut ()).unwrap();

        // we get the actual payload that needs to be forwarded to #2
        let (address, payload) = r1out.recv().unwrap();
        assert_eq!(&addr2, &address);
        assert_eq!(&b"hello".to_vec(), &payload);
        s2in.send((addr1.clone(), payload)).unwrap();

        t2.process(&mut ()).unwrap();

        // #2 now gets the payload!
        let mut msg_list = t2.drain_messages();
        let msg = msg_list.get_mut(2).unwrap().take_message();
        if let Some(RequestToParent::ReceivedData { address, payload }) = msg {
            assert_eq!(&address, &addr1full);
            assert_eq!(&b"hello".to_vec(), &payload);
        } else {
            panic!("bad type {:?}", msg);
        }
    }
}