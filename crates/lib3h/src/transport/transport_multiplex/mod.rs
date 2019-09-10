//! Let's say we have two agentIds: a1 and a2
//! a1 is running on machineId: m1
//! a2 is running on machineId: m2
//!
//! The AgentSpaceGateway will wrap messages in a p2p_proto direct message:
//!   DirectMessage {
//!     space_address: "Qmyada",
//!     to_agent_id: "a2",
//!     from_agent_id: "a1",
//!     payload: <...>,
//!   }
//!
//! Then send it to the machine id:
//!   dest: "m2", payload: <above, but binary>
//!
//! When the multiplexer receives data (at the network/machine gateway),
//! if it is any other p2p_proto message, it will be forwarded to
//! the engine or network gateway. If it is a direct message, it will be
//! sent to the appropriate Route / AgentSpaceGateway

mod mplex;
pub use mplex::TransportMultiplex;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::{error::*, protocol::*};
    use detach::prelude::*;
    use lib3h_ghost_actor::prelude::*;
    use url::Url;

    enum MockToParentContext {}

    pub struct TransportMock {
        endpoint_parent: Option<TransportActorParentEndpoint>,
        endpoint_self: Detach<
            GhostContextEndpoint<
                TransportMock,
                MockToParentContext,
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
                        self.mock_sender.send((address, payload))?;
                        msg.respond(Ok(RequestToChildResponse::SendMessage))?;
                    }
                }
            }
            loop {
                match self.mock_receiver.try_recv() {
                    Ok((address, payload)) => {
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
    fn it_should_multiplex() {
        let (s_out, r_out) = crossbeam_channel::unbounded();
        let (s_in, r_in) = crossbeam_channel::unbounded();

        let addr_none = Url::parse("none:").expect("can parse url");

        let mut mplex: TransportActorParentWrapper<(), (), TransportMultiplex> =
            GhostParentWrapper::new(
                TransportMultiplex::new(Box::new(TransportMock::new(s_out, r_in))),
                "test_mplex_",
            );

        let mut route_a = mplex
            .as_mut()
            .create_agent_space_route(&"space_a".into(), &"agent_a".into())
            .as_context_endpoint_builder()
            .build::<(), ()>();

        let mut route_b = mplex
            .as_mut()
            .create_agent_space_route(&"space_b".into(), &"agent_b".into())
            .as_context_endpoint_builder()
            .build::<(), ()>();

        // send a message from route A
        route_a
            .request(
                (),
                RequestToChild::SendMessage {
                    address: addr_none.clone(),
                    payload: b"hello-from-a".to_vec(),
                },
                Box::new(|_, _, response| {
                    assert_eq!(&format!("{:?}", response), "");
                    Ok(())
                }),
            )
            .unwrap();

        route_a.process(&mut ()).unwrap();
        mplex.process(&mut ()).unwrap();
        route_a.process(&mut ()).unwrap();
        mplex.process(&mut ()).unwrap();

        // should receive that out the bottom
        let (address, payload) = r_out.recv().unwrap();
        assert_eq!(&addr_none, &address);
        assert_eq!(&b"hello-from-a".to_vec(), &payload);

        // send a message up the bottom
        s_in.send((addr_none.clone(), b"hello-to-b".to_vec()))
            .unwrap();

        // process "receive" that message
        mplex.process(&mut ()).unwrap();
        let mut msgs = mplex.drain_messages();
        assert_eq!(1, msgs.len());

        let msg = msgs.remove(0).take_message().unwrap();
        if let RequestToParent::ReceivedData { address, payload } = msg {
            assert_eq!(&addr_none, &address);
            assert_eq!(&b"hello-to-b".to_vec(), &payload);
        } else {
            panic!("bad type");
        }

        // our mplex module got it, now we should have the context
        // let's instruct it to be forwarded up the route
        mplex
            .as_mut()
            .received_data_for_agent_space_route(
                &"space_b".into(),
                &"agent_b".into(),
                &"agent_x".into(),
                &"machine_x".into(),
                b"hello".to_vec(),
            )
            .unwrap();

        mplex.process(&mut ()).unwrap();
        route_b.process(&mut ()).unwrap();

        let mut msgs = route_b.drain_messages();
        assert_eq!(1, msgs.len());

        let msg = msgs.remove(0).take_message().unwrap();
        if let RequestToParent::ReceivedData { address, payload } = msg {
            assert_eq!(
                &Url::parse("transportid:machine_x?a=agent_x").unwrap(),
                &address
            );
            assert_eq!(&b"hello".to_vec(), &payload);
        } else {
            panic!("bad type");
        }
    }
}