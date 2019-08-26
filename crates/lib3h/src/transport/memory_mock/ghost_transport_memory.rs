use crate::transport::{error::TransportError, memory_mock::memory_server, protocol::*};
use lib3h_ghost_actor::{GhostActor, GhostActorState, RequestId, WorkWasDone};
use std::{
    any::Any,
    collections::HashSet,
    sync::{Arc, Mutex},
};
use url::Url;

#[derive(Debug)]
#[allow(dead_code)]
enum RequestToParentContext {
    Source { address: Url },
}

type GhostTransportMemoryState = GhostActorState<
    RequestToParentContext,
    RequestToParent,
    RequestToParentResponse,
    RequestToChildResponse,
    TransportError,
>;

#[allow(dead_code)]
struct GhostTransportMemory {
    actor_state: Option<GhostTransportMemoryState>,
    /// My peer uri on the network layer (not None after a bind)
    maybe_my_address: Option<Url>,
    /// Addresses of connections to remotes
    connections: HashSet<Url>,
}

impl GhostTransportMemory {
    #[allow(dead_code)]
    pub fn new() -> Self {
        let mut tc = TRANSPORT_COUNT
            .lock()
            .expect("could not lock transport count mutex");
        *tc += 1;
        Self {
            actor_state: Some(GhostActorState::new()),
            maybe_my_address: None,
            connections: HashSet::new(),
        }
    }

    fn respond_with(&mut self, request_id: &Option<RequestId>, response: RequestToChildResponse) {
        if let Some(request_id) = request_id {
            self.get_actor_state()
                .respond_to_parent(request_id.clone(), response);
        }
    }
}

macro_rules! is_bound {
    ($self:ident, $request_id:ident, $response_type:ident  ) => {
        match &mut $self.maybe_my_address {
            Some(my_addr) => my_addr.clone(),
            None => {
                $self.respond_with(
                    &$request_id,
                    RequestToChildResponse::$response_type(Err(TransportError::new(
                        "Transport must be bound before sending".to_string(),
                    ))),
                );
                return;
            }
        }
    };
}

/*
macro_rules! with_server {
    ($self:ident, $request_id:ident, $response_type:ident, $address:ident, |$server:ident| $code:expr  ) => {
        let server_map = memory_server::MEMORY_SERVER_MAP.read().unwrap();
        let maybe_server = server_map.get(&$address);
        if let None = maybe_server {
            respond_with!($self,$request_id,$response_type,
                          Err(TransportError::new(format!(
                              "No Memory server at this url address: {}",
                              $address
                          ))));
            return;
        }
        let mut server = maybe_server.unwrap().lock().unwrap();
        $code
    }
}
 */

impl
    GhostActor<
        RequestToParentContext,
        RequestToParent,
        RequestToParentResponse,
        RequestToChild,
        RequestToChildResponse,
        TransportError,
    > for GhostTransportMemory
{
    // BOILERPLATE START----------------------------------

    fn as_any(&mut self) -> &mut dyn Any {
        &mut *self
    }

    fn get_actor_state(&mut self) -> &mut GhostTransportMemoryState {
        self.actor_state.as_mut().unwrap()
    }

    fn take_actor_state(&mut self) -> GhostTransportMemoryState {
        std::mem::replace(&mut self.actor_state, None).unwrap()
    }

    fn put_actor_state(&mut self, actor_state: GhostTransportMemoryState) {
        std::mem::replace(&mut self.actor_state, Some(actor_state));
    }

    // BOILERPLATE END----------------------------------

    // our parent is making a request of us
    //#[allow(irrefutable_let_patterns)]
    fn request(&mut self, request_id: Option<RequestId>, request: RequestToChild) {
        match request {
            RequestToChild::Bind { spec: _url } => {
                // get a new bound url from the memory server (we ignore the spec here)
                let bound_url = memory_server::new_url();
                memory_server::set_server(&bound_url).unwrap(); //set_server always returns Ok
                self.maybe_my_address = Some(bound_url.clone());

                // respond to our parent
                self.respond_with(
                    &request_id,
                    RequestToChildResponse::Bind(Ok(BindResultData {
                        bound_url: bound_url,
                    })),
                );
            }
            RequestToChild::SendMessage { address, payload } => {
                // make sure we have bound and get our address if so
                let my_addr = is_bound!(self, request_id, SendMessage);

                // get destinations server
                let server_map = memory_server::MEMORY_SERVER_MAP.read().unwrap();
                let maybe_server = server_map.get(&address);
                if let None = maybe_server {
                    self.respond_with(
                        &request_id,
                        RequestToChildResponse::SendMessage(Err(TransportError::new(format!(
                            "No Memory server at this address: {}",
                            my_addr
                        )))),
                    );
                    return;
                }
                let mut server = maybe_server.unwrap().lock().unwrap();

                // if not already connected, request a connections
                if self.connections.get(&address).is_none() {
                    let result = server.request_connect(&my_addr);
                    if result.is_err() {
                        self.respond_with(&request_id, RequestToChildResponse::SendMessage(result));
                        return;
                    }
                    self.connections.insert(address.clone());
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

                self.respond_with(&request_id, RequestToChildResponse::SendMessage(Ok(())));
            }
        }
    }

    fn process_concrete(&mut self) -> Result<WorkWasDone, TransportError> {
        // make sure we have bound and get our address if so
        let my_addr = match &self.maybe_my_address {
            Some(my_addr) => my_addr.clone(),
            None => return Ok(false.into()),
        };

        println!("Processing for: {}", my_addr);

        // get our own server
        let server_map = memory_server::MEMORY_SERVER_MAP.read().unwrap();
        let maybe_server = server_map.get(&my_addr);
        if let None = maybe_server {
            return Err(TransportError::new(format!(
                "No Memory server at this address: {}",
                my_addr
            )));
        }
        let mut server = maybe_server.unwrap().lock().unwrap();
        let (success, event_list) = server.process()?;
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
                        self.get_actor_state().send_event_to_parent(
                            RequestToParent::IncomingConnection {
                                address: to_connect_uri.clone(),
                            },
                        );
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
                        println!("RecivedData--- from:{:?} payload:{:?}", from_addr, payload);
                        self.get_actor_state().send_event_to_parent(
                            RequestToParent::ReceivedData {
                                address: Url::parse(&from_addr).unwrap(),
                                payload,
                            },
                        );
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

lazy_static! {
    /// Counter of the number of GhostTransportMemory that spawned
    static ref TRANSPORT_COUNT: Arc<Mutex<u32>> = Arc::new(Mutex::new(0));
}

#[cfg(test)]
mod tests {

    use super::*;
    use lib3h_ghost_actor::RequestId;
    // use protocol::RequestToChildResponse;

    #[test]
    fn test_gmem_transport() {
        let mut transport1 = GhostTransportMemory::new();
        let mut transport2 = GhostTransportMemory::new();

        // create two memory bindings so that we have addresses
        let bind_request1 = RequestId::with_prefix("test_parent");
        let bind_request2 = RequestId::with_prefix("test_parent");

        assert_eq!(transport1.maybe_my_address, None);
        assert_eq!(transport2.maybe_my_address, None);

        transport1.request(
            Some(bind_request1),
            RequestToChild::Bind {
                spec: Url::parse("mem://_").unwrap(),
            },
        );
        transport2.request(
            Some(bind_request2),
            RequestToChild::Bind {
                spec: Url::parse("mem://_").unwrap(),
            },
        );

        let expected_transport1_address = Url::parse("mem://addr_1").unwrap();
        assert_eq!(
            transport1.maybe_my_address,
            Some(expected_transport1_address.clone())
        );
        let mut r1 = transport1.drain_responses();
        let (_rid, response) = r1.pop().unwrap();
        match response {
            RequestToChildResponse::Bind(Ok(bind_result)) => {
                // the memory transport server should bind us to the first available url which is a1
                assert_eq!(bind_result.bound_url, expected_transport1_address);
            }
            _ => assert!(false),
        }

        let expected_transport2_address = Url::parse("mem://addr_2").unwrap();
        assert_eq!(
            transport2.maybe_my_address,
            Some(expected_transport2_address.clone())
        );
        let mut r2 = transport2.drain_responses();
        let (_rid, response) = r2.pop().unwrap();
        match response {
            RequestToChildResponse::Bind(Ok(bind_result)) => {
                // the memory transport server should bind us to the first available url which is a1
                assert_eq!(bind_result.bound_url, expected_transport2_address);
            }
            _ => assert!(false),
        }

        // now send a message from transport1 to transport2 over the bound addresses
        let send_request1 = RequestId::with_prefix("test_parent");
        transport1.request(
            Some(send_request1),
            RequestToChild::SendMessage {
                address: Url::parse("mem://addr_2").unwrap(),
                payload: b"test message".to_vec(),
            },
        );

        // call process on both transports so queues can fill
        transport1.process().unwrap();
        transport2.process().unwrap();

        let requests = transport2.drain_requests();
        assert_eq!(
            "[(None, IncomingConnection { address: \"mem://addr_1/\" }), (None, ReceivedData { address: \"mem://addr_1/\", payload: [116, 101, 115, 116, 32, 109, 101, 115, 115, 97, 103, 101] })]",
            format!("{:?}", requests)
        );

        let mut r = transport1.drain_responses();
        let (_rid, response) = r.pop().unwrap();
        assert_eq!("SendMessage(Ok(()))", format!("{:?}", response));
    }
}
