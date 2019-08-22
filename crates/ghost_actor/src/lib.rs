extern crate nanoid;
#[macro_use]
extern crate shrinkwraprs;

#[derive(Shrinkwrap, Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[shrinkwrap(mutable)]
pub struct DidWork(pub bool);

impl From<bool> for DidWork {
    fn from(b: bool) -> Self {
        DidWork(b)
    }
}

impl From<DidWork> for bool {
    fn from(d: DidWork) -> Self {
        d.0
    }
}

#[derive(Shrinkwrap, Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[shrinkwrap(mutable)]
pub struct RequestId(pub String);

impl RequestId {
    pub fn new() -> Self {
        Self::with_prefix("")
    }

    pub fn with_prefix(prefix: &str) -> Self {
        Self(format!("{}{}", prefix, nanoid::simple()))
    }
}

impl From<String> for RequestId {
    fn from(s: String) -> Self {
        RequestId(s)
    }
}

impl From<RequestId> for String {
    fn from(r: RequestId) -> Self {
        r.0
    }
}

mod ghost_tracker;
pub use ghost_tracker::{GhostCallback, GhostCallbackData, GhostTracker};

mod ghost_actor_state;
pub use ghost_actor_state::GhostActorState;

mod ghost_actor;
pub use ghost_actor::GhostActor;

pub mod prelude {
    pub use super::{GhostActor, GhostActorState, GhostCallback, GhostCallbackData, GhostTracker};
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::any::Any;

    #[allow(dead_code)]
    mod transport_protocol {
        #[derive(Debug)]
        pub enum RequestToChild {
            Bind { url: String },
        }

        #[derive(Debug)]
        pub enum RequestToChildResponse {
            BindResult { bound_url: Result<String, String> },
        }

        #[derive(Debug)]
        pub enum RequestToParent {
            IncomingConnection { address: String },
        }

        #[derive(Debug)]
        pub enum RequestToParentResponse {
            Allowed,
            Disallowed,
        }
    }

    use transport_protocol::*;

    struct WssTransport {
        actor_state: Option<
            GhostActorState<
                RequestToParent,
                RequestToParentResponse,
                RequestToChildResponse,
                String,
            >,
        >,
    }

    impl WssTransport {
        pub fn new() -> Self {
            Self {
                actor_state: Some(GhostActorState::new()),
            }
        }
    }

    impl
        GhostActor<
            RequestToParent,
            RequestToParentResponse,
            RequestToChild,
            RequestToChildResponse,
            String,
        > for WssTransport
    {
        fn as_any(&mut self) -> &mut dyn Any {
            &mut *self
        }

        fn get_actor_state(
            &mut self,
        ) -> &mut GhostActorState<
            RequestToParent,
            RequestToParentResponse,
            RequestToChildResponse,
            String,
        > {
            self.actor_state.as_mut().unwrap()
        }

        fn take_actor_state(
            &mut self,
        ) -> GhostActorState<RequestToParent, RequestToParentResponse, RequestToChildResponse, String>
        {
            std::mem::replace(&mut self.actor_state, None).unwrap()
        }

        fn put_actor_state(
            &mut self,
            actor_state: GhostActorState<
                RequestToParent,
                RequestToParentResponse,
                RequestToChildResponse,
                String,
            >,
        ) {
            std::mem::replace(&mut self.actor_state, Some(actor_state));
        }

        // our parent is making a request of us
        fn request(&mut self, request_id: Option<RequestId>, request: RequestToChild) {
            match request {
                RequestToChild::Bind { url: _u } => {
                    // do some internal bind
                    // we get a bound_url
                    let bound_url = "bound_url".to_string();
                    // respond to our parent
                    if let Some(request_id) = request_id {
                        self.get_actor_state().respond_to_parent(
                            request_id,
                            RequestToChildResponse::BindResult {
                                bound_url: Ok(bound_url),
                            },
                        );
                    }
                }
            }
        }

        fn process_concrete(&mut self) -> Result<DidWork, String> {
            self.get_actor_state().send_request_to_parent(
                std::time::Duration::from_millis(2000),
                RequestToParent::IncomingConnection {
                    address: "test".to_string(),
                },
                Box::new(|_m, r| {
                    println!("response from parent to IncomingConnection got: {:?}", r);
                    Ok(())
                }),
            );
            Ok(true.into())
        }
    }

    type TransportActor = dyn GhostActor<
        RequestToParent,
        RequestToParentResponse,
        RequestToChild,
        RequestToChildResponse,
        String,
    >;
    use crate::RequestId;

    #[test]
    fn test_wss_transport() {
        // the body of this test simulates an object that contains a actor, i.e. a parent.
        // it would usually just be another ghost_actor but here we test it out explicitly
        // so first instantiate the "child" actor
        let mut t_actor: Box<TransportActor> = Box::new(WssTransport::new());

        // allow the actor to run this actor always creates a simulated incoming
        // connection each time it processes
        t_actor.process().unwrap();

        // now process any requests the actor may have made of us (as parent)
        for (rid, ev) in t_actor.drain_requests() {
            println!("in drain_requests got: {:?} {:?}", rid, ev);
            if let Some(rid) = rid {
                // we might allow or disallow connections for example
                let response = RequestToParentResponse::Allowed;
                t_actor.respond(rid, response).unwrap();
            }
        }

        // now make a request of the child,
        // to make such a request the parent would normally will also instantiate trackers so that it can
        // handle responses when they come back as callbacks.
        // here we simply watch that we got a response back as expected
        let request_id = RequestId::with_prefix("test_parent");
        t_actor.request(
            Some(request_id),
            RequestToChild::Bind {
                url: "address_to_bind_to".to_string(),
            },
        );

        // now process the responses the actor has made to our requests
        for (rid, ev) in t_actor.drain_responses() {
            println!("in drain_responses got: {:?} {:?}", rid, ev);
        }
    }
}