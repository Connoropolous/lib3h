use crate::{
    dht::{dht_config::DhtConfig, dht_protocol::*},
    engine::GatewayId,
    gateway::P2pGateway,
    message_encoding::*,
    transport,
};
use detach::prelude::*;
use lib3h_ghost_actor::prelude::*;
use url::Url;

//--------------------------------------------------------------------------------------------------
// Constructors
//--------------------------------------------------------------------------------------------------

/// P2pGateway Constructors
impl P2pGateway {
    /// Constructor
    /// Bind and set advertise on construction by using the name as URL.
    pub fn new(
        wrap_dm: bool,
        identifier: GatewayId,
        peer_uri: Url,
        inner_transport: transport::protocol::DynTransportActor,
        dht_factory: DhtFactory,
        dht_config: &DhtConfig,
    ) -> Self {
        let dht = dht_factory(dht_config).expect("Failed to construct DHT");
        let (endpoint_parent, endpoint_self) = create_ghost_channel();
        let endpoint_self = Detach::new(
            endpoint_self
                .as_context_endpoint_builder()
                .request_id_prefix(&format!("{}_to_parent_", identifier.nickname))
                .build(),
        );
        P2pGateway {
            wrap_dm,
            identifier: identifier,
            inner_transport: Detach::new(transport::protocol::TransportActorParentWrapperDyn::new(
                inner_transport,
                "to_child_transport_",
            )),
            inner_dht: Detach::new(ChildDhtWrapperDyn::new(dht, "gateway_dht_")),
            message_encoding: Detach::new(GhostParentWrapper::new(
                MessageEncoding::new(),
                "to_message_encoding_",
            )),
            endpoint_parent: Some(endpoint_parent),
            endpoint_self,
            this_peer: PeerData {
                peer_address: dht_config.this_peer_address(),
                peer_uri,
                timestamp: 0, // FIXME
            },
            pending_outgoing_messages: Vec::new(),
        }
    }

    pub fn this_peer(&self) -> PeerData {
        self.this_peer.clone()
    }
}
