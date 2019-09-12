#![allow(non_snake_case)]

use lib3h_tracing::Lib3hTrace;
use std::collections::{HashMap, HashSet, VecDeque};
use url::Url;

use super::RealEngineTrackerData;
use crate::{
    dht::{dht_config::DhtConfig, dht_protocol::*},
    engine::{
        p2p_protocol::*, ChainId, RealEngine, RealEngineConfig, TransportKeys, NETWORK_GATEWAY_ID,
    },
    error::{Lib3hError, Lib3hResult},
    gateway::{protocol::*, P2pGateway},
    track::Tracker,
    transport::{self, memory_mock::ghost_transport_memory::*, TransportMultiplex},
};
use detach::prelude::*;
use lib3h_crypto_api::{Buffer, CryptoSystem};
use lib3h_ghost_actor::prelude::*;
use lib3h_protocol::{
    data_types::*, error::Lib3hProtocolResult, network_engine::NetworkEngine,
    protocol_client::Lib3hClientProtocol, protocol_server::Lib3hServerProtocol, Address, DidWork,
};

use rmp_serde::Serializer;
use serde::Serialize;

impl TransportKeys {
    pub fn new(crypto: &dyn CryptoSystem) -> Lib3hResult<Self> {
        let hcm0 = hcid::HcidEncoding::with_kind("hcm0")?;
        let mut public_key: Box<dyn Buffer> = Box::new(vec![0; crypto.sign_public_key_bytes()]);
        let mut secret_key = crypto.buf_new_secure(crypto.sign_secret_key_bytes());
        crypto.sign_keypair(&mut public_key, &mut secret_key)?;
        Ok(Self {
            transport_id: hcm0.encode(&public_key)?,
            transport_public_key: public_key,
            transport_secret_key: secret_key,
        })
    }
}

// FIXME TransportWss
//impl RealEngine {
//    /// Constructor with TransportWss
//    pub fn new(
//        crypto: Box<dyn CryptoSystem>,
//        config: RealEngineConfig,
//        name: &str,
//        dht_factory: DhtFactory,
//    ) -> Lib3hResult<Self> {
//        // Create Transport and bind
//        let network_transport =
//            TransportWrapper::new(TransportWss::with_std_tcp_stream(config.tls_config.clone()));
//        let binding = network_transport.as_mut().bind(&config.bind_url)?;
//        // Generate keys
//        // TODO #209 - Check persistence first before generating
//        let transport_keys = TransportKeys::new(crypto.as_crypto_system())?;
//        // Generate DHT config and create multiplexer
//        let dht_config = DhtConfig {
//            this_peer_address: transport_keys.transport_id.clone(),
//            this_peer_uri: binding,
//            custom: config.dht_custom_config.clone(),
//            gossip_interval: config.dht_gossip_interval,
//            timeout_threshold: config.dht_timeout_threshold,
//        };
//        let multiplexer = GatewayParentWrapperDyn::new(P2pGateway::new(
//            NETWORK_GATEWAY_ID,
//            network_transport.clone(),
//            dht_factory,
//            &dht_config,
//        ));
//        // Done
//        Ok(RealEngine {
//            crypto,
//            config,
//            inbox: VecDeque::new(),
//            name: name.to_string(),
//            dht_factory,
//            request_track: Tracker::new("real_engine_", 2000),
//            network_transport,
//            multiplexer,
//            network_connections: HashSet::new(),
//            space_gateway_map: HashMap::new(),
//            transport_keys,
//            process_count: 0,
//            temp_outbox: Vec::new(),
//        })
//    }
//}

/// Constructor
impl RealEngine {
    /// Constructor with TransportMemory
    pub fn new_mock(
        crypto: Box<dyn CryptoSystem>,
        config: RealEngineConfig,
        name: &str,
        dht_factory: DhtFactory,
    ) -> Lib3hResult<Self> {
        // Create TransportMemory as the network transport
        let mut memory_transport = GhostTransportMemory::new();
        let memory_network_endpoint = Detach::new(
            memory_transport
                .take_parent_endpoint()
                .expect("exists")
                .as_context_endpoint_builder()
                .request_id_prefix("tmem_to_child_")
                .build::<P2pGateway, Lib3hTrace>(),
        );

        //        // Bind & create this_net_peer
        //        // TODO: Find better way to do init with GhostEngine
        //        let mut gateway_ud = GatewayUserData::new();
        //        let _res = memory_network_endpoint.request(
        //            Lib3hTrace,
        //            transport::protocol::RequestToChild::Bind {
        //                spec: config.bind_url.clone(),
        //            },
        //            Box::new(|mut ud, response| {
        //                let response = {
        //                    match response {
        //                        GhostCallbackData::Timeout => panic!("timeout"),
        //                        GhostCallbackData::Response(response) => match response {
        //                            Err(e) => panic!("{:?}", e),
        //                            Ok(response) => response,
        //                        },
        //                    }
        //                };
        //                if let transport::protocol::RequestToChildResponse::Bind(bind_data) = response {
        //                    ud.binding = bind_data.bound_url;
        //                } else {
        //                    panic!("bad response to bind: {:?}", response);
        //                }
        //                Ok(())
        //            }),
        //        );
        //        memory_transport.process()?;
        //        memory_network_endpoint.process(&mut gateway_ud)?;

        let fixme_binding = Url::parse("fixme::host:123").unwrap();
        let this_net_peer = PeerData {
            peer_address: format!("{}_tId", name),
            peer_uri: fixme_binding.clone(),
            timestamp: 0, // TODO #166
        };
        // Create DhtConfig
        let dht_config =
            DhtConfig::with_real_engine_config(&format!("{}_tId", name), &fixme_binding, &config);
        debug!("New MOCK RealEngine {} -> {:?}", name, this_net_peer);
        let transport_keys = TransportKeys::new(crypto.as_crypto_system())?;
        let multiplexer = Detach::new(GatewayParentWrapper::new(
            TransportMultiplex::new(P2pGateway::new(
                NETWORK_GATEWAY_ID,
                memory_network_endpoint,
                dht_factory,
                &dht_config,
            )),
            "to_multiplexer_",
        ));
        let mut real_engine = RealEngine {
            crypto,
            config,
            inbox: VecDeque::new(),
            name: name.to_string(),
            dht_factory,
            request_track: Tracker::new("real_engine_", 2000),
            multiplexer,
            this_net_peer,
            network_connections: HashSet::new(),
            space_gateway_map: HashMap::new(),
            transport_keys,
            process_count: 0,
            temp_outbox: Vec::new(),
        };
        real_engine.priv_connect_bootstraps()?;
        Ok(real_engine)
    }

    fn priv_connect_bootstraps(&mut self) -> Lib3hProtocolResult<()> {
        // TODO
        let nodes: Vec<Url> = self.config.bootstrap_nodes.drain(..).collect();
        for bs in nodes {
            self.post(Lib3hClientProtocol::Connect(ConnectData {
                request_id: format!("bootstrap-connect: {}", bs.clone()).to_string(), // fire-and-forget
                peer_uri: bs,
                network_id: "".to_string(), // unimplemented
            }))?;
        }
        Ok(())
    }

    pub fn this_space_peer(&mut self, chain_id: ChainId) -> PeerData {
        trace!("engine.this_space_peer() ...");
        let mut space_gateway = self
            .space_gateway_map
            .remove(&chain_id)
            .expect("No space at chainId");
        space_gateway.as_mut().as_mut().this_peer()
    }
}

impl NetworkEngine for RealEngine {
    /// User provided identifier for this engine
    fn name(&self) -> String {
        self.name.clone()
    }

    fn advertise(&self) -> Url {
        self.this_net_peer.peer_uri.to_owned()
    }

    /// Add incoming Lib3hClientProtocol message in FIFO
    fn post(&mut self, client_msg: Lib3hClientProtocol) -> Lib3hProtocolResult<()> {
        // trace!("RealEngine.post(): {:?}", client_msg);
        self.inbox.push_back(client_msg);
        Ok(())
    }

    /// Process Lib3hClientProtocol message inbox and
    /// output a list of Lib3hServerProtocol messages for Core to handle
    fn process(&mut self) -> Lib3hProtocolResult<(DidWork, Vec<Lib3hServerProtocol>)> {
        self.process_count += 1;
        trace!("");
        trace!("{} - process() START - {}", self.name, self.process_count);
        // Process all received Lib3hClientProtocol messages from Core
        let (inbox_did_work, mut outbox) = self.process_inbox()?;
        // Process the network layer
        let (net_did_work, mut net_outbox) = self.process_multiplexer()?;
        outbox.append(&mut net_outbox);
        // Process the space layer
        let mut p2p_output = self.process_space_gateways()?;
        outbox.append(&mut p2p_output);

        // Hack
        let (ugly_did_work, mut ugly_outbox) = self.process_ugly();
        outbox.append(&mut ugly_outbox);

        trace!(
            "process() END - {} (outbox: {})\n",
            self.process_count,
            outbox.len(),
        );

        for (timeout_id, timeout_data) in self.request_track.process_timeouts() {
            error!("timeout {:?} {:?}", timeout_id, timeout_data);
        }

        // Done
        Ok((inbox_did_work || net_did_work || ugly_did_work, outbox))
    }
}

/// Drop
impl Drop for RealEngine {
    fn drop(&mut self) {
        self.shutdown().unwrap_or_else(|e| {
            warn!("Graceful shutdown failed: {}", e);
        });
    }
}

/// Private
impl RealEngine {
    /// Called on drop.
    /// Close all connections gracefully
    fn shutdown(&mut self) -> Lib3hResult<()> {
        let /*mut*/ result: Lib3hResult<()> = Ok(());

        // FIXME
        //        for space_gatway in self.space_gateway_map.values_mut() {
        //            let res = space_gatway.as_transport_mut().close_all();
        //            // Continue closing connections even if some failed
        //            if let Err(e) = res {
        //                if result.is_ok() {
        //                    result = Err(e.into());
        //                }
        //            }
        //        }
        //        self.multiplexer
        //            .as_transport_mut()
        //            .close_all()
        //            .map_err(|e| {
        //                error!("Closing of some connection failed: {:?}", e);
        //                e
        //            })?;

        result
    }

    /// Progressively serve every Lib3hClientProtocol received in inbox
    fn process_inbox(&mut self) -> Lib3hResult<(DidWork, Vec<Lib3hServerProtocol>)> {
        let mut outbox = Vec::new();
        let did_work = self.inbox.len() > 0;
        loop {
            let client_msg = match self.inbox.pop_front() {
                None => break,
                Some(msg) => msg,
            };
            let mut output = self.serve_Lib3hClientProtocol(client_msg)?;
            outbox.append(&mut output);
        }
        Ok((did_work, outbox))
    }

    /// Process a Lib3hClientProtocol message sent to us (by Core)
    /// Side effects: Might add other messages to sub-components' inboxes.
    /// Return a list of Lib3hServerProtocol messages to send back to core or others?
    fn serve_Lib3hClientProtocol(
        &mut self,
        client_msg: Lib3hClientProtocol,
    ) -> Lib3hResult<Vec<Lib3hServerProtocol>> {
        debug!("{} serving: {:?}", self.name, client_msg);
        let mut outbox = Vec::new();
        // Note: use same order as the enum
        match client_msg {
            Lib3hClientProtocol::Shutdown => {
                // TODO
            }
            Lib3hClientProtocol::SuccessResult(_msg) => {
                // TODO #168
            }
            Lib3hClientProtocol::FailureResult(_msg) => {
                // TODO #168
            }
            Lib3hClientProtocol::Connect(msg) => {
                // no-op ?
                // send empty message so it connects
                let cmd = GatewayRequestToChild::Transport(
                    transport::protocol::RequestToChild::SendMessage {
                        uri: msg.peer_uri,
                        payload: Opaque::new(),
                    },
                );
                // TODO: Figure out how we want to handle Connect and ConnectResult with GhostEngine
                self.multiplexer.publish(cmd)?;
                //                // Convert into TransportCommand & post to network gateway
                //                let cmd = TransportCommand::Connect(msg.peer_uri, msg.request_id);
                //                self.multiplexer.as_transport_mut().post(cmd)?;
            }
            Lib3hClientProtocol::JoinSpace(msg) => {
                let mut output = self.serve_JoinSpace(&msg)?;
                outbox.append(&mut output);
            }
            Lib3hClientProtocol::LeaveSpace(msg) => {
                let srv_msg = self.serve_LeaveSpace(&msg);
                outbox.push(srv_msg);
            }
            Lib3hClientProtocol::SendDirectMessage(msg) => {
                let srv_msg = self.serve_DirectMessage(msg, false);
                outbox.push(srv_msg);
            }
            Lib3hClientProtocol::HandleSendDirectMessageResult(msg) => {
                let srv_msg = self.serve_DirectMessage(msg, true);
                outbox.push(srv_msg);
            }
            Lib3hClientProtocol::FetchEntry(_msg) => {
                // TODO #169
            }
            // HandleFetchEntryResult:
            //   - From GetAuthoringList      : Convert to DhtCommand::BroadcastEntry
            //   - From DHT EntryDataRequested: Convert to DhtCommand::EntryDataResponse
            Lib3hClientProtocol::HandleFetchEntryResult(msg) => {
                let mut is_data_for_author_list = false;
                if self.request_track.has(&msg.request_id) {
                    match self.request_track.remove(&msg.request_id) {
                        Some(data) => match data {
                            RealEngineTrackerData::DataForAuthorEntry => {
                                is_data_for_author_list = true;
                            }
                            _ => (),
                        },
                        None => (),
                    };
                }
                let maybe_space = self.get_space_or_fail(
                    &msg.space_address,
                    &msg.provider_agent_id,
                    &msg.request_id,
                    None,
                );
                match maybe_space {
                    Err(res) => outbox.push(res),
                    Ok(space_gateway) => {
                        let dht_request = if is_data_for_author_list {
                            DhtRequestToChild::BroadcastEntry(msg.entry)
                        } else {
                            DhtRequestToChild::HoldEntryAspectAddress(msg.entry)
                        };
                        let _ = space_gateway.publish(GatewayRequestToChild::Dht(dht_request));
                    }
                }
            }
            // PublishEntry: Broadcast on the space DHT
            Lib3hClientProtocol::PublishEntry(msg) => {
                // MIRROR - reflecting hold for now
                for aspect in &msg.entry.aspect_list {
                    let msg = StoreEntryAspectData {
                        request_id: self.request_track.reserve(),
                        space_address: msg.space_address.clone(),
                        provider_agent_id: msg.provider_agent_id.clone(),
                        entry_address: msg.entry.entry_address.clone(),
                        entry_aspect: aspect.clone(),
                    };
                    self.request_track.set(
                        &msg.request_id,
                        Some(RealEngineTrackerData::HoldEntryRequested),
                    );
                    outbox.push(Lib3hServerProtocol::HandleStoreEntryAspect(msg));
                }
                let maybe_space = self.get_space_or_fail(
                    &msg.space_address,
                    &msg.provider_agent_id,
                    &format!("PublishEntry_{:?}", msg.entry.entry_address),
                    None,
                );
                match maybe_space {
                    Err(res) => outbox.push(res),
                    Ok(space_gateway) => {
                        let _ = space_gateway.publish(GatewayRequestToChild::Dht(
                            DhtRequestToChild::BroadcastEntry(msg.entry),
                        ));
                    }
                }
            }
            // HoldEntry: Core validated an entry/aspect and tells us its holding it.
            Lib3hClientProtocol::HoldEntry(msg) => {
                let maybe_space = self.get_space_or_fail(
                    &msg.space_address,
                    &msg.provider_agent_id,
                    &format!("HoldEntry_{:?}", msg.entry.entry_address),
                    None,
                );
                match maybe_space {
                    Err(res) => outbox.push(res),
                    Ok(space_gateway) => {
                        let _ = space_gateway.publish(GatewayRequestToChild::Dht(
                            DhtRequestToChild::HoldEntryAspectAddress(msg.entry),
                        ));
                    }
                }
            }
            // TODO #169
            Lib3hClientProtocol::QueryEntry(msg) => {
                let maybe_space = self.get_space_or_fail(
                    &msg.space_address,
                    &msg.requester_agent_id,
                    &msg.request_id,
                    None,
                );
                match maybe_space {
                    Err(res) => outbox.push(res),
                    Ok(_space_gateway) => {
                        // TODO #169 reflecting for now...
                        // ultimately this should get forwarded to the
                        // correct neighborhood
                        outbox.push(Lib3hServerProtocol::HandleQueryEntry(msg))
                    }
                }
            }
            // TODO #169
            Lib3hClientProtocol::HandleQueryEntryResult(msg) => {
                let maybe_space = self.get_space_or_fail(
                    &msg.space_address,
                    &msg.responder_agent_id,
                    &msg.request_id,
                    None,
                );
                match maybe_space {
                    Err(res) => outbox.push(res),
                    Ok(_space_gateway) => {
                        // TODO #169 reflecting for now...
                        // ultimately this should get forwarded to the
                        // original requestor
                        outbox.push(Lib3hServerProtocol::QueryEntryResult(msg))
                    }
                }
            }
            // Our request for the publish_list has returned
            Lib3hClientProtocol::HandleGetAuthoringEntryListResult(msg) => {
                self.serve_HandleGetAuthoringEntryListResult(&mut outbox, msg)?;
            }
            // Our request for the hold_list has returned
            Lib3hClientProtocol::HandleGetGossipingEntryListResult(msg) => {
                self.serve_HandleGetGossipingEntryListResult(&mut outbox, msg)?;
            }
        }
        Ok(outbox)
    }

    fn serve_HandleGetAuthoringEntryListResult(
        &mut self,
        _outbox: &mut Vec<Lib3hServerProtocol>,
        _msg: EntryListData,
    ) -> Lib3hResult<()> {
        // FIXME
        //        if !self.request_track.has(&msg.request_id) {
        //            error!("untracked HandleGetAuthoringEntryListResult");
        //        } else {
        //            match self.request_track.remove(&msg.request_id) {
        //                Some(data) => match data {
        //                    RealEngineTrackerData::GetAuthoringEntryList => (),
        //                    _ => error!("bad track type HandleGetAuthoringEntryListResult"),
        //                },
        //                None => error!("bad track type HandleGetAuthoringEntryListResult"),
        //            };
        //        }
        //        let maybe_space = self.get_space_or_fail(
        //            &msg.space_address,
        //            &msg.provider_agent_id,
        //            &msg.request_id,
        //            None,
        //        );
        //        if let Err(res) = maybe_space {
        //            outbox.push(res);
        //            return Ok(());
        //        }
        //        let space_gateway = maybe_space.unwrap();
        //        // Request every 'new' Entry from Core
        //        for (entry_address, aspect_address_list) in msg.address_map.clone() {
        //            // Check aspects and only request entry with new aspects
        //            space_gateway.request(
        //                GatewayContext::Dht(DhtContext::RequestAspectsOf {
        //                    entry_address: entry_address.clone(),
        //                    aspect_address_list,
        //                    msg: msg.clone(),
        //                    request_id: self.request_track.reserve(),
        //                }),
        //                GatewayRequestToChild::Dht(DhtRequestToChild::RequestAspectsOf(entry_address.clone())),
        //                Box::new(|ud, context, response| {
        //                    let (entry_address, aspect_address_list, msg, request_id) = {
        //                        if let DhtContext::RequestAspectsOf {
        //                            entry_address,
        //                            aspect_address_list,
        //                            msg,
        //                            request_id,
        //                        } = context
        //                        {
        //                            (entry_address, aspect_address_list, msg, request_id)
        //                        } else {
        //                            panic!("bad context type");
        //                        }
        //                    };
        //                    let response = {
        //                        match response {
        //                            GhostCallbackData::Timeout => panic!("timeout"),
        //                            GhostCallbackData::Response(response) => match response {
        //                                Err(e) => panic!("{:?}", e),
        //                                Ok(response) => response,
        //                            },
        //                        }
        //                    };
        //                    if let DhtRequestToChildResponse::RequestAspectsOf(maybe_known_aspects) =
        //                        response
        //                    {
        //                        let can_fetch = match maybe_known_aspects {
        //                            None => true,
        //                            Some(known_aspects) => {
        //                                let can = !includes(&known_aspects, &aspect_address_list);
        //                                can
        //                            }
        //                        };
        //                        if can_fetch {
        //                            let msg_data = FetchEntryData {
        //                                space_address: msg.space_address.clone(),
        //                                entry_address: entry_address.clone(),
        //                                request_id,
        //                                provider_agent_id: msg.provider_agent_id.clone(),
        //                                aspect_address_list: None,
        //                            };
        //                            ud.lib3h_outbox
        //                                .push(Lib3hServerProtocol::HandleFetchEntry(msg_data));
        //                        }
        //                    } else {
        //                        panic!("bad response to RequestAspectsOf: {:?}", response);
        //                    }
        //                    Ok(())
        //                }),
        //            )?;
        //        }
        Ok(())
    }

    fn process_ugly(&mut self) -> (DidWork, Vec<Lib3hServerProtocol>) {
        trace!("process_ugly() - {}", self.temp_outbox.len());
        let mut outbox = Vec::new();
        let mut did_work = false;
        for srv_msg in self.temp_outbox.drain(0..) {
            did_work = true;
            if let Lib3hServerProtocol::HandleFetchEntry(msg) = srv_msg.clone() {
                self.request_track.set(
                    &msg.request_id,
                    Some(RealEngineTrackerData::DataForAuthorEntry),
                );
            }
            outbox.push(srv_msg);
        }
        trace!("process_ugly() END - {}", outbox.len());
        (did_work, outbox)
    }

    fn serve_HandleGetGossipingEntryListResult(
        &mut self,
        outbox: &mut Vec<Lib3hServerProtocol>,
        msg: EntryListData,
    ) -> Lib3hResult<()> {
        if !self.request_track.has(&msg.request_id) {
            error!("untracked HandleGetGossipingEntryListResult");
        } else {
            match self.request_track.remove(&msg.request_id) {
                Some(data) => match data {
                    RealEngineTrackerData::GetGossipingEntryList => (),
                    _ => error!("bad track type HandleGetGossipingEntryListResult"),
                },
                None => error!("bad track type HandleGetGossipingEntryListResult"),
            };
        }
        let maybe_space = self.get_space_or_fail(
            &msg.space_address,
            &msg.provider_agent_id,
            &msg.request_id,
            None,
        );
        match maybe_space {
            Err(res) => outbox.push(res),
            Ok(_space_gateway) => {
                for (entry_address, _aspect_address_list) in msg.address_map {
                    // #fullsync hack
                    // fetch every entry from owner
                    self.request_track.set(
                        &msg.request_id,
                        Some(RealEngineTrackerData::DataForAuthorEntry),
                    );
                    let msg_data = FetchEntryData {
                        space_address: msg.space_address.clone(),
                        entry_address: entry_address.clone(),
                        request_id: self.request_track.reserve(),
                        provider_agent_id: msg.provider_agent_id.clone(),
                        aspect_address_list: None,
                    };
                    outbox.push(Lib3hServerProtocol::HandleFetchEntry(msg_data));
                }
            }
        }
        Ok(())
    }

    /// Create a gateway for this agent in this space, if not already part of it.
    /// Must not already be part of this space.
    fn serve_JoinSpace(&mut self, join_msg: &SpaceData) -> Lib3hResult<Vec<Lib3hServerProtocol>> {
        // Prepare response
        let mut res = GenericResultData {
            request_id: join_msg.request_id.clone(),
            space_address: join_msg.space_address.clone(),
            to_agent_id: join_msg.agent_id.clone(),
            result_info: vec![].into(),
        };
        // Bail if space already joined by agent
        let chain_id = (join_msg.space_address.clone(), join_msg.agent_id.clone());
        if self.space_gateway_map.contains_key(&chain_id) {
            res.result_info = "Already joined space".to_string().into();
            return Ok(vec![Lib3hServerProtocol::FailureResult(res)]);
        }
        let mut output = Vec::new();
        output.push(Lib3hServerProtocol::SuccessResult(res));
        // First create DhtConfig for space gateway
        let agent_id: String = join_msg.agent_id.clone().into();
        let this_peer_transport_id_as_uri = {
            // TODO #175 - encapsulate this conversion logic
            Url::parse(format!("transportId:{}", self.this_net_peer.peer_address).as_str())
                .expect("can parse url")
        };
        let dht_config = DhtConfig::with_real_engine_config(
            &agent_id,
            &this_peer_transport_id_as_uri,
            &self.config,
        );
        // Create new space gateway for this ChainId
        let uniplex_endpoint = Detach::new(
            self.multiplexer
                .as_mut()
                .as_mut()
                .create_agent_space_route(&join_msg.space_address, &agent_id.into())
                .as_context_endpoint_builder()
                .build::<P2pGateway, Lib3hTrace>(),
        );
        let new_space_gateway = Detach::new(GatewayParentWrapper::new(
            P2pGateway::new_with_space(
                &join_msg.space_address,
                uniplex_endpoint,
                self.dht_factory,
                &dht_config,
            ),
            "space_gateway_",
        ));

        // TODO #150 - Send JoinSpace to all known peers
        let space_address: String = join_msg.space_address.clone().into();
        let peer = self.this_space_peer(chain_id.clone()).to_owned();
        let mut payload = Vec::new();
        let p2p_msg = P2pProtocol::BroadcastJoinSpace(space_address.clone(), peer.clone());
        p2p_msg
            .serialize(&mut Serializer::new(&mut payload))
            .unwrap();
        trace!(
            "{} - Broadcasting JoinSpace: {}, {}",
            self.name,
            space_address,
            peer.peer_address,
        );
        self.multiplexer
            .publish(GatewayRequestToChild::SendAll(payload))?;
        // TODO END

        // Add it to space map
        self.space_gateway_map
            .insert(chain_id.clone(), new_space_gateway);

        // Have DHT broadcast our PeerData
        let space_gateway = self.space_gateway_map.get_mut(&chain_id).unwrap();
        let this_peer = peer.clone(); // FIXME
        space_gateway.publish(GatewayRequestToChild::Dht(DhtRequestToChild::HoldPeer(
            this_peer,
        )))?;

        // Send Get*Lists requests
        let mut list_data = GetListData {
            space_address: join_msg.space_address.clone(),
            provider_agent_id: join_msg.agent_id.clone(),
            request_id: self.request_track.reserve(),
        };
        self.request_track.set(
            &list_data.request_id,
            Some(RealEngineTrackerData::GetGossipingEntryList),
        );
        output.push(Lib3hServerProtocol::HandleGetGossipingEntryList(
            list_data.clone(),
        ));
        list_data.request_id = self.request_track.reserve();
        self.request_track.set(
            &list_data.request_id,
            Some(RealEngineTrackerData::GetAuthoringEntryList),
        );
        output.push(Lib3hServerProtocol::HandleGetAuthoringEntryList(list_data));
        // Done
        Ok(output)
    }

    fn serve_DirectMessage(
        &mut self,
        msg: DirectMessageData,
        is_response: bool,
    ) -> Lib3hServerProtocol {
        // get sender's peer address
        let chain_id = (msg.space_address.clone(), msg.from_agent_id.clone());
        let peer_address = self.this_space_peer(chain_id).peer_address.clone();
        // Check if space is joined by sender
        let maybe_space = self.get_space_or_fail(
            &msg.space_address,
            &msg.from_agent_id,
            &msg.request_id,
            None,
        );
        // Return failure if not
        if let Err(failure_msg) = maybe_space {
            return failure_msg;
        }
        let space_gateway = maybe_space.unwrap();
        // Prepare response
        let mut response = GenericResultData {
            request_id: msg.request_id.clone(),
            space_address: msg.space_address.clone(),
            to_agent_id: msg.from_agent_id.clone(),
            result_info: vec![].into(),
        };
        // Check if messaging self
        let to_agent_id: String = msg.to_agent_id.clone().into();
        if peer_address == to_agent_id {
            response.result_info = "Messaging self".as_bytes().to_vec().into();
            return Lib3hServerProtocol::FailureResult(response);
        }
        // Change into P2pProtocol
        let net_msg = if is_response {
            P2pProtocol::DirectMessageResult(msg.clone())
        } else {
            P2pProtocol::DirectMessage(msg.clone())
        };
        // Serialize payload
        let mut payload = Vec::new();
        net_msg
            .serialize(&mut Serializer::new(&mut payload))
            .unwrap();
        // Send
        let peer_address: String = msg.to_agent_id.clone().into();
        let _res = space_gateway.publish(GatewayRequestToChild::Transport(
            transport::protocol::RequestToChild::SendMessage {
                uri: Url::parse(&("agentId:".to_string() + &peer_address))
                    .expect("invalid url format"),
                payload: payload.into(),
            },
        ));
        Lib3hServerProtocol::SuccessResult(response)
    }

    /// Destroy gateway for this agent in this space, if part of it.
    /// Respond with FailureResult if space was not already joined.
    fn serve_LeaveSpace(&mut self, join_msg: &SpaceData) -> Lib3hServerProtocol {
        // Try remove
        let chain_id = (join_msg.space_address.clone(), join_msg.agent_id.clone());
        let res = self.space_gateway_map.remove(&chain_id);
        // Create response according to remove result
        let response = GenericResultData {
            request_id: join_msg.request_id.clone(),
            space_address: join_msg.space_address.clone(),
            to_agent_id: join_msg.agent_id.clone(),
            result_info: match res {
                None => "Agent is not part of the space"
                    .to_string()
                    .into_bytes()
                    .into(),
                Some(_) => vec![].into(),
            },
        };
        // Done
        match res {
            None => Lib3hServerProtocol::FailureResult(response),
            Some(_) => Lib3hServerProtocol::SuccessResult(response),
        }
    }

    /// Get a space_gateway for the specified space+agent.
    /// If agent did not join that space, respond with a FailureResult instead.
    fn get_space_or_fail(
        &mut self,
        space_address: &Address,
        agent_id: &Address,
        request_id: &str,
        maybe_sender_agent_id: Option<&Address>,
    ) -> Result<&mut GatewayParentWrapper<RealEngine, Lib3hTrace, P2pGateway>, Lib3hServerProtocol>
    {
        let maybe_space = self
            .space_gateway_map
            .get_mut(&(space_address.to_owned(), agent_id.to_owned()));
        if let Some(space_gateway) = maybe_space {
            return Ok(space_gateway);
        }
        let to_agent_id = maybe_sender_agent_id.unwrap_or(agent_id);
        let res = GenericResultData {
            request_id: request_id.to_string(),
            space_address: space_address.to_owned(),
            to_agent_id: to_agent_id.to_owned(),
            result_info: format!(
                "Agent {} does not track space {}",
                &agent_id, &space_address,
            )
            .as_bytes()
            .to_vec()
            .into(),
        };
        Err(Lib3hServerProtocol::FailureResult(res))
    }
}

pub fn handle_gossipTo<
    G: GhostActor<
        GatewayRequestToParent,
        GatewayRequestToParentResponse,
        GatewayRequestToChild,
        GatewayRequestToChildResponse,
        Lib3hError,
    >,
>(
    gateway_identifier: &str,
    gateway: &mut GatewayParentWrapper<RealEngine, Lib3hTrace, G>,
    gossip_data: GossipToData,
) -> Lib3hResult<()> {
    debug!(
        "({}) handle_gossipTo: {:?}",
        gateway_identifier, gossip_data,
    );

    for to_peer_address in gossip_data.peer_address_list {
        // FIXME
        //            // TODO #150 - should not gossip to self in the first place
        //            let me = self.get_this_peer_sync(&mut gateway).peer_address;
        //            if to_peer_address == me {
        //                continue;
        //            }
        //            // TODO END

        // Convert DHT Gossip to P2P Gossip
        let p2p_gossip = P2pProtocol::Gossip(GossipData {
            space_address: gateway_identifier.into(),
            to_peer_address: to_peer_address.clone().into(),
            from_peer_address: "FIXME".into(), // FIXME
            bundle: gossip_data.bundle.clone(),
        });
        let mut payload = Vec::new();
        p2p_gossip
            .serialize(&mut Serializer::new(&mut payload))
            .expect("P2pProtocol::Gossip serialization failed");
        // Forward gossip to the inner_transport
        // FIXME peer_address to Url convert
        let msg = transport::protocol::RequestToChild::SendMessage {
            uri: Url::parse(&("agentId:".to_string() + &to_peer_address)).expect("invalid Url"),
            payload: payload.into(),
        };
        gateway.publish(GatewayRequestToChild::Transport(msg))?;
    }
    Ok(())
}

/// Return true if all elements of list_b are found in list_a
#[allow(dead_code)]
fn includes(list_a: &[Address], list_b: &[Address]) -> bool {
    let set_a: HashSet<_> = list_a.iter().map(|addr| addr).collect();
    let set_b: HashSet<_> = list_b.iter().map(|addr| addr).collect();
    set_b.is_subset(&set_a)
}
