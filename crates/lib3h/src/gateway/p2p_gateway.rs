#![allow(non_snake_case)]

use crate::{
    dht::{
        dht_protocol::*,
        dht_trait::{Dht, DhtConfig, DhtFactory},
    },
    gateway::{self, P2pGateway},
    transport::transport_trait::Transport,
};
use lib3h_protocol::{Address, Lib3hResult};
use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    rc::Rc,
};

/// Public interface
impl<T: Transport, D: Dht> P2pGateway<T, D> {
    /// This Gateway's identifier
    pub fn identifier(&self) -> &str {
        self.identifier.as_str()
    }
    /// TODO #179  - using an explicit post for dht because of dumb rust compiler
    pub fn post_dht(&mut self, cmd: DhtCommand) -> Lib3hResult<()> {
        if let DhtCommand::HoldPeer(peer_data) = cmd.clone() {
            let previous = self.connection_map.insert(
                peer_data.peer_uri.clone(),
                gateway::url_to_transport_id(&peer_data.peer_uri.clone()),
            );
            if let Some(previous_cId) = previous {
                debug!(
                    "Replaced connectionId[{}] = {}",
                    peer_data.peer_uri.clone(),
                    previous_cId
                );
            }
        }
        self.inner_dht.post(cmd)
    }
}

//--------------------------------------------------------------------------------------------------
// Constructors
//--------------------------------------------------------------------------------------------------

/// any Transport Constructor
impl<T: Transport, D: Dht> P2pGateway<T, D> {
    /// Constructor
    /// Bind and set advertise on construction by using the name as URL.
    pub fn new(
        identifier: &str,
        inner_transport: Rc<RefCell<T>>,
        dht_factory: DhtFactory<D>,
        dht_config: &DhtConfig,
    ) -> Self {
        P2pGateway {
            inner_transport,
            inner_dht: dht_factory(dht_config).expect("Failed to construct DHT"),
            identifier: identifier.to_owned(),
            connection_map: HashMap::new(),
            transport_inbox: VecDeque::new(),
        }
    }
}

/// P2pGateway Constructor
impl<T: Transport, D: Dht> P2pGateway<P2pGateway<T, D>, D> {
    /// Constructors
    pub fn new_with_space(
        network_gateway: Rc<RefCell<P2pGateway<T, D>>>,
        space_address: &Address,
        dht_factory: DhtFactory<D>,
        dht_config: &DhtConfig,
    ) -> Self {
        let identifier: String = space_address.clone().into();
        P2pGateway {
            inner_transport: network_gateway,
            inner_dht: dht_factory(dht_config).expect("Failed to construct DHT"),
            identifier,
            connection_map: HashMap::new(),
            transport_inbox: VecDeque::new(),
        }
    }
}
