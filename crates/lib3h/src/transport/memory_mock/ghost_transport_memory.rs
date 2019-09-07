use crate::transport::{error::TransportError, memory_mock::memory_server, protocol::*};
use lib3h_ghost_actor::prelude::*;
use std::collections::HashSet;
use url::Url;

#[derive(Debug)]
#[allow(dead_code)]
enum RequestToParentContext {
    Source { address: Url },
}

pub type UserData = GhostTransportMemory;

type GhostTransportMemoryEndpoint = GhostEndpoint<
    RequestToChild,
    RequestToChildResponse,
    RequestToParent,
    RequestToParentResponse,
    TransportError,
>;

type GhostTransportMemoryEndpointContext = GhostContextEndpoint<
    UserData,
    (),
    RequestToParent,
    RequestToParentResponse,
    RequestToChild,
    RequestToChildResponse,
    TransportError,
>;

pub type GhostTransportMemoryEndpointContextParent = GhostContextEndpoint<
    (),
    (),
    RequestToChild,
    RequestToChildResponse,
    RequestToParent,
    RequestToParentResponse,
    TransportError,
>;

#[allow(dead_code)]
pub struct GhostTransportMemory {
    endpoint_parent: Option<GhostTransportMemoryEndpoint>,
    endpoint_self: Option<GhostTransportMemoryEndpointContext>,
    /// My peer uri on the network layer (not None after a bind)
    maybe_my_address: Option<Url>,
    /// Addresses of connections to remotes
    connections: HashSet<Url>,
    server_refs: std::collections::HashMap<Url, memory_server::ServerInst>,
}

impl GhostTransportMemory {
    #[allow(dead_code)]
    pub fn new() -> Self {
        let (endpoint_parent, endpoint_self) = create_ghost_channel();
        Self {
            endpoint_parent: Some(endpoint_parent),
            endpoint_self: Some(
                endpoint_self
                    .as_context_endpoint_builder()
                    .request_id_prefix("tmem_to_parent")
                    .build(),
            ),
            connections: HashSet::new(),
            maybe_my_address: None,
            server_refs: std::collections::HashMap::new(),
        }
    }
}

impl From<TransportError> for GhostError {
    fn from(e: TransportError) -> Self {
        format!("TransportError: {}", e).into()
    }
}

impl
    GhostActor<
        RequestToParent,
        RequestToParentResponse,
        RequestToChild,
        RequestToChildResponse,
        TransportError,
    > for GhostTransportMemory
{
    // BOILERPLATE START----------------------------------

    fn take_parent_endpoint(&mut self) -> Option<GhostTransportMemoryEndpoint> {
        std::mem::replace(&mut self.endpoint_parent, None)
    }

    // BOILERPLATE END----------------------------------

    fn process_concrete(&mut self) -> GhostResult<WorkWasDone> {
        trace!("process concrete");
        // process the self endpoint
        let mut endpoint_self = std::mem::replace(&mut self.endpoint_self, None);
        endpoint_self.as_mut().expect("exists").process(self)?;
        std::mem::replace(&mut self.endpoint_self, endpoint_self);

        for mut msg in self
            .endpoint_self
            .as_mut()
            .expect("exists")
            .drain_messages()
        {
            trace!("msg take");

            match msg.take_message().expect("exists") {
                RequestToChild::Bind { spec: _url } => {
                    trace!("Bind");
                    // get a new bound url from the memory server (we ignore the spec here)
                    let bound_url = memory_server::new_url();
                    // memory_server::ensure_server(&bound_url.clone()).expect("bound server"); //set_server always returns Ok

                    self.server_refs.insert(
                        bound_url.clone(),
                        memory_server::ensure_server(&bound_url.clone()).expect("bound server"),
                    ); //set_server always returns Ok

                    self.maybe_my_address = Some(bound_url.clone());

                    // respond to our parent
                    msg.respond(Ok(RequestToChildResponse::Bind(BindResultData {
                        bound_url: bound_url,
                    })))?;
                }
                RequestToChild::SendMessage { address, payload } => {
                    trace!("SendMessage");
                    // make sure we have bound and get our address if so
                    //let my_addr = is_bound!(self, request_id, SendMessage);

                    // make sure we have bound and get our address if so
                    match &self.maybe_my_address {
                        None => {
                            msg.respond(Err(TransportError::new(
                                "Transport must be bound before sending".to_string(),
                            )))?;
                        }
                        Some(my_addr) => {
                            let expected = format!("server for address {:?}", address);
                            let maybe_server =
                                memory_server::read_ref(&address).expect(expected.as_str());
                            let mut server = maybe_server.get();
                            /*                            // get destinations server
                                                        // TODO propagate error
                                                        if let Err(e) = maybe_server {
                                                            println!("server error: {:?}", e);
                                                            msg.respond(Err(TransportError::new(format!(
                                                                "No Memory server at this address: {}",
                                                                my_addr
                                                            ))))?;
                                                            continue;
                                                        }

                                                        let server_ref = maybe_server.unwrap();
                                                        let mut server = server_ref.get();
                            */
                            // if not already connected, request a connections
                            if self.connections.get(&address).is_none() {
                                match server.request_connect(&my_addr) {
                                    Err(err) => {
                                        msg.respond(Err(err))?;
                                        continue;
                                    }
                                    Ok(()) => self.connections.insert(address.clone()),
                                };
                            };

                            trace!(
                                "(GhostTransportMemory).SendMessage from {} to  {} | {:?}",
                                my_addr,
                                address,
                                payload
                            );
                            // Send it data from us
                            server
                                .post(&my_addr, &payload)
                                .expect("Post on memory server should work");

                            msg.respond(Ok(RequestToChildResponse::SendMessage))?;
                        }
                    };
                }
            }
        }

        // make sure we have bound and get our address if so
        let my_addr = match &self.maybe_my_address {
            Some(my_addr) => my_addr.clone(),
            None => return Ok(false.into()),
        };

        trace!("Processing for: {}", my_addr);

        // get our own server
        let maybe_server = memory_server::read_ref(&my_addr);
        if let Err(e) = maybe_server {
            println!("error: {}", e);
            return Err(format!("No Memory server at this address: {}", my_addr).into());
        }
        let server_ref = maybe_server.unwrap();
        let mut server = server_ref.get();

        trace!("Invoking process");
        let (success, event_list) = server.process()?;
        trace!("Invoked process");
        if success {
            let mut to_connect_list: Vec<(Url)> = Vec::new();
            let mut non_connect_events = Vec::new();

            // process any connection events
            for event in event_list {
                match event {
                    TransportEvent::IncomingConnectionEstablished(in_cid) => {
                        let to_connect_uri =
                            Url::parse(&in_cid).expect("connectionId is not a valid Url");
                        to_connect_list.push(to_connect_uri.clone());
                        let mut endpoint_self = std::mem::replace(&mut self.endpoint_self, None);
                        endpoint_self.as_mut().expect("exists").publish(
                            RequestToParent::IncomingConnection {
                                address: to_connect_uri.clone(),
                            },
                        )?;
                        std::mem::replace(&mut self.endpoint_self, endpoint_self);
                    }
                    _ => non_connect_events.push(event),
                }
            }

            // Connect back to received connections if not already connected to them
            for remote_addr in to_connect_list {
                println!(
                    "(GhostTransportMemory)connecting {} <- {:?}",
                    remote_addr, my_addr
                );

                // if not already connected, request a connections
                if self.connections.get(&remote_addr).is_none() {
                    let _result = server.request_connect(&remote_addr);
                    self.connections.insert(remote_addr.clone());
                }
            }

            // process any other events
            for event in non_connect_events {
                match event {
                    TransportEvent::ReceivedData(from_addr, payload) => {
                        trace!("ReceivedData--- from:{:?} payload:{:?}", from_addr, payload);
                        let mut endpoint_self = std::mem::replace(&mut self.endpoint_self, None);
                        endpoint_self.as_mut().expect("exists").publish(
                            RequestToParent::ReceivedData {
                                address: Url::parse(&from_addr).unwrap(),
                                payload,
                            },
                        )?;
                        std::mem::replace(&mut self.endpoint_self, endpoint_self);
                    }
                    _ => panic!(format!("WHAT: {:?}", event)),
                };
            }
            Ok(true.into())
        } else {
            Ok(false.into())
        }
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    //use protocol::RequestToChildResponse;
    //    use lib3h_ghost_actor::GhostCallbackData;
    #[allow(dead_code)]
    fn enable_logging_for_test(enable: bool) {
        // wait a bit because of non monotonic clock,
        // otherwise we could get negative substraction panics
        // TODO #211
        std::thread::sleep(std::time::Duration::from_millis(10));
        if std::env::var("RUST_LOG").is_err() {
            std::env::set_var("RUST_LOG", "debug");
        }
        let _ = env_logger::builder()
            .default_format_timestamp(false)
            .default_format_module_path(false)
            .is_test(enable)
            .try_init();
    }

    #[test]
    fn test_gmem_transport() {
        enable_logging_for_test(true);
        /* Possible other ways we might think of setting up
               constructors for actor/parent_context_endpoint pairs:

            let (transport1_endpoint, child) = ghost_create_endpoint();
            let transport1_engine = GhostTransportMemoryEngine::new(child);

            enum TestContex {
        }

            let mut transport1_actor = GhostLocalActor::new::<TestContext>(
            transport1_engine, transport1_endpoint);
             */

        /*
            let mut transport1 = GhostParentContextEndpoint::with_cb(|child| {
            GhostTransportMemory::new(child)
        });

            let mut transport1 = GhostParentContextEndpoint::new(
            Box::new(GhostTransportMemory::new()));
             */

        let mut transport1 = GhostTransportMemory::new();
        let mut t1_endpoint: GhostTransportMemoryEndpointContextParent = transport1
            .take_parent_endpoint()
            .expect("exists")
            .as_context_endpoint_builder()
            .request_id_prefix("tmem_to_child1")
            .build::<(), ()>();

        let mut transport2 = GhostTransportMemory::new();
        let mut t2_endpoint = transport2
            .take_parent_endpoint()
            .expect("exists")
            .as_context_endpoint_builder()
            .request_id_prefix("tmem_to_child2")
            .build::<(), ()>();

        // create two memory bindings so that we have addresses
        assert_eq!(transport1.maybe_my_address, None);
        assert_eq!(transport2.maybe_my_address, None);

        let expected_transport1_address = Url::parse("mem://addr_1").unwrap();
        t1_endpoint
            .request(
                (),
                RequestToChild::Bind {
                    spec: Url::parse("mem://_").unwrap(),
                },
                Box::new(|_: &mut (), _, r| {
                    // parent should see the bind event
                    assert_eq!(
                        "Response(Ok(Bind(BindResultData { bound_url: \"mem://addr_1/\" })))",
                        &format!("{:?}", r)
                    );
                    Ok(())
                }),
            )
            .unwrap();
        trace!("t1 endpoint requested bind");
        let expected_transport2_address = Url::parse("mem://addr_2").unwrap();
        t2_endpoint
            .request(
                (),
                RequestToChild::Bind {
                    spec: Url::parse("mem://_").unwrap(),
                },
                Box::new(|_: &mut (), _, r| {
                    // parent should see the bind event
                    assert_eq!(
                        "Response(Ok(Bind(BindResultData { bound_url: \"mem://addr_2/\" })))",
                        &format!("{:?}", r)
                    );
                    Ok(())
                }),
            )
            .unwrap();
        trace!("t2 endpoint requested bind");
        transport1.process().unwrap();
        let _ = t1_endpoint.process(&mut ());

        transport2.process().unwrap();
        let _ = t2_endpoint.process(&mut ());

        trace!("assert_eq1");
        assert_eq!(
            transport1.maybe_my_address,
            Some(expected_transport1_address)
        );
        assert_eq!(
            transport2.maybe_my_address,
            Some(expected_transport2_address)
        );
        trace!("send message");

        let cb = Box::new(|_: &mut (), _, r| {
            println!("HERE!");
            // parent should see that the send request was OK
            assert_eq!("Response(Ok(SendMessage))", &format!("{:?}", r));
            Ok(())
        });

        trace!("send message2");

        let send_message = RequestToChild::SendMessage {
            address: Url::parse("mem://addr_2").unwrap(),
            payload: "test message".into(),
        };

        println!("send message3");
        // now send a message from transport1 to transport2 over the bound addresses
        let result = t1_endpoint.request((), send_message, cb);
        trace!("send message3");

        assert!(result.is_ok());
        trace!("sent message");

        trace!("transport1.process");
        transport1.process().unwrap();
        let _ = t1_endpoint.process(&mut ());

        trace!("transport2.process");
        transport2.process().unwrap();
        let _ = t2_endpoint.process(&mut ());

        let mut requests = t2_endpoint.drain_messages();
        assert_eq!(2, requests.len());
        assert_eq!(
            "Some(IncomingConnection { address: \"mem://addr_1/\" })",
            format!("{:?}", requests[0].take_message())
        );
        assert_eq!(
            "Some(ReceivedData { address: \"mem://addr_1/\", payload: \"test message\" })",
            format!("{:?}", requests[1].take_message())
        );
    }
}
