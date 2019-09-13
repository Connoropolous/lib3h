use crate::{
    dht::{dht_config::DhtConfig, dht_protocol::*},
    gateway::P2pGateway,
    transport,
};
use detach::prelude::*;
use lib3h_ghost_actor::prelude::*;
use lib3h_protocol::Address;

//--------------------------------------------------------------------------------------------------
// Constructors
//--------------------------------------------------------------------------------------------------

/// P2pGateway Constructors
impl P2pGateway {
    /// Constructor
    /// Bind and set advertise on construction by using the name as URL.
    pub fn new(
        identifier: &str,
        child_transport_endpoint: transport::protocol::DynTransportActor,
        dht_factory: DhtFactory,
        dht_config: &DhtConfig,
    ) -> Self {
        let dht = dht_factory(dht_config).expect("Failed to construct DHT");
        let (endpoint_parent, endpoint_self) = create_ghost_channel();
        let endpoint_self = Detach::new(
            endpoint_self
                .as_context_endpoint_builder()
                .request_id_prefix(&format!("{}_to_parent_", identifier))
                .build(),
        );
        P2pGateway {
            identifier: identifier.to_owned(),
            child_transport_endpoint: Detach::new(
                transport::protocol::TransportActorParentWrapperDyn::new(
                    child_transport_endpoint,
                    "to_child_transport_",
                ),
            ),
            inner_dht: Detach::new(ChildDhtWrapperDyn::new(dht, "gateway_dht")),
            endpoint_parent: Some(endpoint_parent),
            endpoint_self,
            this_peer: PeerData {
                peer_address: dht_config.this_peer_address(),
                peer_uri: dht_config.this_peer_uri(),
                timestamp: 0, // FIXME
            },
        }
    }
    /// Helper Ctor
    pub fn new_with_space(
        space_address: &Address,
        child_transport_endpoint: transport::protocol::DynTransportActor,
        dht_factory: DhtFactory,
        dht_config: &DhtConfig,
    ) -> Self {
        let identifier: String = space_address.clone().into();
        P2pGateway::new(
            &identifier,
            child_transport_endpoint,
            dht_factory,
            dht_config,
        )
    }

    pub fn this_peer(&self) -> PeerData {
        self.this_peer.clone()
    }
}
//
//impl P2pGateway {
//    // FIXME
//    pub fn drain_dht_outbox(&mut self) -> Vec<Lib3hServerProtocol> {
//        self.user_data.lib3h_outbox.drain(0..).collect()
//    }
//}
