#![allow(non_snake_case)]

use crate::{
    dht::dht_protocol::*,
    engine::p2p_protocol::P2pProtocol,
    error::*,
    gateway::{
        protocol::*, GatewayOutputWrapType, P2pGateway, send_data_types::*,
    },
    message_encoding::encoding_protocol,
    transport::{self, error::TransportResult},
};
use holochain_tracing::Span;
use lib3h_ghost_actor::prelude::*;
use lib3h_protocol::{data_types::*, uri::Lib3hUri, Address};
use rmp_serde::{Deserializer, Serializer};
use serde::{Deserialize, Serialize};

/// Private internals
impl P2pGateway {
    /// Handle IncomingConnection event from child transport
    fn handle_incoming_connection(&mut self, span: Span, uri: Lib3hUri) -> TransportResult<()> {
        self.inner_dht.request(
            span.child("handle_incoming_connection"),
            DhtRequestToChild::RequestThisPeer,
            Box::new(move |me, response| {
                let response = {
                    match response {
                        GhostCallbackData::Timeout(bt) => panic!("timeout: {:?}", bt),
                        GhostCallbackData::Response(response) => match response {
                            Err(e) => panic!("{:?}", e),
                            Ok(response) => response,
                        },
                    }
                };
                if let DhtRequestToChildResponse::RequestThisPeer(this_peer) = response {
                    // once we have the peer info from the other side, bubble the incoming connection
                    // to the network layer
                    me.endpoint_self.publish(
                        Span::fixme(),
                        GatewayRequestToParent::Transport(
                            transport::protocol::RequestToParent::IncomingConnection {
                                uri: this_peer.peer_name.clone(),
                            },
                        ),
                    )?;

                    // Send to other node our PeerName
                    let our_peer_name = P2pProtocol::PeerName(
                        me.identifier.id.to_owned().into(),
                        this_peer.peer_name,
                        this_peer.timestamp,
                    );
                    let mut buf = Vec::new();
                    our_peer_name
                        .serialize(&mut Serializer::new(&mut buf))
                        .unwrap();
                    trace!(
                        "({}) sending P2pProtocol::PeerName: {:?} to {:?}",
                        me.identifier.nickname,
                        our_peer_name,
                        uri,
                    );
                    me.send_with_full_low_uri(
                        SendWithFullLowUri {
                            span: span.follower("TODO send"),
                            full_low_uri: uri.clone(),
                            payload: buf.into(),
                        },
                        Box::new(|response| {
                            match response {
                                Ok(GatewayRequestToChildResponse::Transport(
                                    transport::protocol::RequestToChildResponse::SendMessageSuccess,
                                )) => {
                                    trace!("Successfully exchanged peer info with new connection")
                                }
                                _ => error!(
                                    "peer info exchange with new connection failed {:?}",
                                    response
                                ),
                            }
                            Ok(())
                        }),
                    )?;
                } else {
                    panic!("bad response to RequestThisPeer: {:?}", response);
                }
                Ok(())
            }),
        )?;
        Ok(())
    }

    #[allow(dead_code)]
    fn priv_decode_on_receive(
        &mut self,
        span: Span,
        uri: Lib3hUri,
        payload: Opaque,
    ) -> GhostResult<()> {
        let e_span = span.child("on_receive");
        self.message_encoding.request(
            span,
            encoding_protocol::RequestToChild::Decode { payload },
            Box::new(move |me, resp| {
                match resp {
                    GhostCallbackData::Response(Ok(
                        encoding_protocol::RequestToChildResponse::DecodeResult {
                            result: encoding_protocol::DecodeData::Payload { payload },
                        },
                    )) => {
                        if payload.len() == 0 {
                            debug!("Implement Ping!");
                        } else {
                            me.priv_on_receive(e_span, uri, payload)?;
                        }
                    }
                    _ => panic!("unexpected decode result: {:?}", resp),
                }
                Ok(())
            }),
        )
    }

    #[allow(dead_code)]
    fn priv_on_receive(&mut self, span: Span, uri: Lib3hUri, payload: Opaque) -> GhostResult<()> {
        let mut de = Deserializer::new(&payload[..]);
        let maybe_p2p_msg: Result<P2pProtocol, rmp_serde::decode::Error> =
            Deserialize::deserialize(&mut de);

        match maybe_p2p_msg {
            Ok(P2pProtocol::PeerName(gateway_id, peer_name, timestamp)) => {
                debug!(
                    "Received PeerName: {} | {} ({})",
                    peer_name, gateway_id, self.identifier.nickname
                );
                if self.identifier.id == gateway_id.into() {
                    let peer = PeerData {
                        peer_name,
                        peer_location: uri.clone(),
                        timestamp,
                    };
                    // HACK
                    self.inner_dht.publish(
                        span.follower("transport::protocol::RequestToParent::ReceivedData"),
                        DhtRequestToChild::HoldPeer(peer),
                    )?;
                }
            }
            Ok(_) => {
                // TODO XXX - nope!
                // We should handle these cases, and pick the ones we want to
                // send up the chain, and which ones should be handled here.

                trace!(
                    "{:?} received {} {}",
                    self.identifier,
                    uri,
                    String::from_utf8_lossy(&payload)
                );

                self.endpoint_self.as_mut().publish(
                    span.follower("bubble up to parent"),
                    GatewayRequestToParent::Transport(
                        transport::protocol::RequestToParent::ReceivedData { uri, payload },
                    ),
                )?;
            }
            _ => {
                panic!(
                    "unexpected received data type {} {:?}",
                    payload, maybe_p2p_msg
                );
            }
        };
        Ok(())
    }

    /*
    fn priv_encoded_send(
        &mut self,
        span: Span,
        to_address: lib3h_protocol::Address,
        uri: Lib3hUri,
        payload: Opaque,
        cb: SendCallback,
    ) -> GhostResult<()> {
        let e_span = span.child("encode_payload");
        self.message_encoding.request(
            span,
            encoding_protocol::RequestToChild::EncodePayload { payload },
            Box::new(move |me, resp| {
                match resp {
                    GhostCallbackData::Response(Ok(
                        encoding_protocol::RequestToChildResponse::EncodePayloadResult { payload },
                    )) => {
                        me.priv_low_level_send(e_span, to_address, uri, payload, cb)?;
                    }
                    _ => {
                        cb(Err(format!(
                            "gateway_transport::priv_encoded_send: {:?}",
                            resp
                        )
                        .into()))?;
                    }
                }
                Ok(())
            }),
        )
    }

    fn priv_low_level_send(
        &mut self,
        span: Span,
        to_address: lib3h_protocol::Address,
        uri: Lib3hUri,
        payload: Opaque,
        cb: SendCallback,
    ) -> GhostResult<()> {
        let payload =
            if let GatewayOutputWrapType::WrapOutputWithP2pDirectMessage = self.wrap_output_type {
                let dm_wrapper = DirectMessageData {
                    space_address: self.identifier.id.clone(),
                    request_id: "".to_string(),
                    to_agent_id: to_address,
                    from_agent_id: self.this_peer.peer_name.clone().into(),
                    content: payload,
                };
                let mut payload = Vec::new();
                let p2p_msg = P2pProtocol::DirectMessage(dm_wrapper);
                p2p_msg
                    .serialize(&mut Serializer::new(&mut payload))
                    .unwrap();
                Opaque::from(payload)
            } else {
                payload
            };

        trace!(
            "({}).priv_low_level_send message from '{}' to '{}'",
            self.identifier.nickname,
            self.this_peer.peer_name.clone(),
            uri.clone()
        );

        // Forward to the child Transport
        self.inner_transport.request(
            span.child("SendMessage"),
            transport::protocol::RequestToChild::SendMessage {
                uri: uri.clone(),
                payload: payload,
                attempt: 0,
            },
            Box::new(move |_me, response| {
                // In case of a transport error or timeout we store the message in the
                // pending list to retry sending it later
                match response {
                    // Success case:
                    GhostCallbackData::Response(Ok(
                        transport::protocol::RequestToChildResponse::SendMessageSuccess,
                    )) => {
                        debug!("Gateway send message successfully");
                        cb(Ok(GatewayRequestToChildResponse::Transport(
                            transport::protocol::RequestToChildResponse::SendMessageSuccess,
                        )))
                    }
                    // No error but something other than SendMessageSuccess:
                    GhostCallbackData::Response(Ok(_)) => {
                        warn!(
                            "Gateway got bad response type from transport: {:?}",
                            response
                        );
                        cb(Err(format!("bad response type: {:?}", response).into()))
                    }
                    // Transport error:
                    GhostCallbackData::Response(Err(error)) => {
                        debug!("Gateway got error from transport. Adding message to pending");
                        cb(Err(format!(
                            "Transport error while trying to send message: {:?}",
                            error
                        )
                        .into()))
                    }
                    // Timeout:
                    GhostCallbackData::Timeout(bt) => {
                        debug!(
                            "Gateway got timeout from transport. Adding message to pending: {:?}",
                            bt
                        );
                        cb(Err(format!(
                            "Ghost timeout error while trying to send message: {:?}",
                            bt
                        )
                        .into()))
                    }
                }
            }),
        )
    }

    /// uri =
    ///   - Network : transportId
    ///   - space   : agentId
    pub(crate) fn send(
        &mut self,
        span: Span,
        to_address: lib3h_protocol::Address,
        uri: Lib3hUri,
        payload: Opaque,
        cb: SendCallback,
    ) -> GhostResult<()> {
        debug!(
            "({}).send() {} | {}",
            self.identifier.nickname,
            uri,
            payload.len()
        );
        self.priv_encoded_send(span, to_address, uri, payload, cb)
    }

    const MAX_RETRY_ATTEMPTS: u8 = 5;
    pub(crate) fn handle_transport_pending_outgoing_messages(&mut self) -> GhostResult<()> {
        let pending: Vec<PendingOutgoingMessage> =
            self.pending_outgoing_messages.drain(..).collect();
        for p in pending {
            let transport_request = transport::protocol::RequestToChild::SendMessage {
                uri: p.uri,
                payload: p.payload,
                attempt: p.attempt + 1,
            };
            self.handle_transport_RequestToChild(p.span, transport_request, p.parent_request)?;
        }
        Ok(())
    }

    fn add_to_pending(
        &mut self,
        span: Span,
        uri: Lib3hUri,
        payload: Opaque,
        parent_request: GatewayToChildMessage,
        attempt: u8,
    ) -> Option<GatewayToChildMessage> {
        if attempt < Self::MAX_RETRY_ATTEMPTS {
            self.pending_outgoing_messages.push(PendingOutgoingMessage {
                span,
                uri,
                payload,
                parent_request,
                attempt,
            });
            trace!(
                "[gateway_transport] add_to_pending, pending_outgoing_messages: {:?}",
                self.pending_outgoing_messages
            );
            None
        } else {
            Some(parent_request)
        }
    }
    */

    /// Handle Transport request sent to use by our parent
    pub(crate) fn handle_transport_RequestToChild(
        &mut self,
        span: Span,
        transport_request: transport::protocol::RequestToChild,
        parent_request: GatewayToChildMessage,
    ) -> Lib3hResult<()> {
        match transport_request {
            transport::protocol::RequestToChild::Bind { spec: _ } => {
                // Forward to child transport
                let _ = self.inner_transport.as_mut().request(
                    span.child("handle_transport_RequestToChild"),
                    transport_request,
                    Box::new(|_me, response| {
                        let response = {
                            match response {
                                GhostCallbackData::Timeout(bt) => {
                                    parent_request
                                        .respond(Err(format!("timeout: {:?}", bt).into()))?;
                                    return Ok(());
                                }
                                GhostCallbackData::Response(response) => response,
                            }
                        };
                        // forward back to parent
                        parent_request.respond(Ok(GatewayRequestToChildResponse::Transport(
                            response.unwrap(),
                        )))?;
                        Ok(())
                    }),
                );
            }
            transport::protocol::RequestToChild::SendMessage {
                uri,
                payload,
                attempt,
            } => {
                debug!(
                    "gateway_transport: SendMessage, first resolving address {:?}",
                    uri.clone()
                );
                // uri is actually a dht peerKey
                // get actual uri from the inner dht before sending
                self.inner_dht.request(
                    span.child("transport::protocol::RequestToChild::SendMessage"),
                    DhtRequestToChild::RequestPeer(uri.clone()),
                    Box::new(move |me, response| {
                        match response {
                            GhostCallbackData::Response(Ok(
                                DhtRequestToChildResponse::RequestPeer(Some(peer_data)),
                            )) => {
                                debug!(
                                    "gateway_transport: address {:?} resolved to {:?}, sending...",
                                    uri.clone(),
                                    peer_data.peer_location.clone()
                                );
                                me.send(
                                    span.follower("TODO send"),
                                    Address::from(peer_data.peer_name),
                                    peer_data.peer_location.clone(),
                                    payload,
                                    Box::new(|response| {
                                        parent_request.respond(
                                            response
                                                .map_err(|transport_error| transport_error.into()),
                                        )
                                    }),
                                )?;
                            }
                            _ => {
                                debug!(
                                    "Couldn't resolve Peer address to send, DHT response was: {:?}",
                                    response
                                );
                                me.add_to_pending(
                                    span.follower("retry_gateway_send"),
                                    uri,
                                    payload,
                                    parent_request,
                                    attempt,
                                )
                                .map(|parent_request| {
                                    parent_request.respond(Err(Lib3hError::from(format!(
                                        "Maximum retries of {:?} already attempted.",
                                        Self::MAX_RETRY_ATTEMPTS
                                    ))))
                                })
                                .unwrap_or_else(|| {
                                    trace!("queued retry for response {:?}", response);
                                    Ok(())
                                })?
                            }
                        };
                        Ok(())
                    }),
                )?;
            }
        }
        // Done
        Ok(())
    }

    /// handle RequestToChildResponse received from child Transport
    /// before forwarding it to our parent
    #[allow(dead_code)]
    pub(crate) fn handle_transport_RequestToChildResponse(
        &mut self,
        response: &transport::protocol::RequestToChildResponse,
    ) -> TransportResult<()> {
        match response {
            transport::protocol::RequestToChildResponse::Bind(_result_data) => {
                // no-op
            }
            transport::protocol::RequestToChildResponse::SendMessageSuccess => {
                // no-op
            }
        };
        Ok(())
    }

    /// Handle request received from child transport
    pub(crate) fn handle_transport_RequestToParent(
        &mut self,
        mut msg: transport::protocol::ToParentMessage,
    ) -> TransportResult<()> {
        trace!(
            "({}) Serving request from child transport: {:?}",
            self.identifier.nickname,
            msg
        );
        let span = msg.span().child("handle_transport_RequestToParent");
        let msg = msg.take_message().expect("exists");
        match &msg {
            transport::protocol::RequestToParent::ErrorOccured { uri: _, error: _ } => {
                // pass any errors back up the chain so network layer can handle them (i.e.)
                self.endpoint_self.publish(
                    Span::fixme(),
                    GatewayRequestToParent::Transport(msg.clone()),
                )?;
            }
            transport::protocol::RequestToParent::IncomingConnection { uri } => {
                // TODO
                info!(
                    "({}) Incoming connection opened: {}",
                    self.identifier.nickname, uri
                );
                self.handle_incoming_connection(
                    span.child("transport::protocol::RequestToParent::IncomingConnection"),
                    uri.clone(),
                )?;
            }
            transport::protocol::RequestToParent::ReceivedData { uri, payload } => {
                // TODO
                trace!(
                    "{:?} Received message from: {} | size: {}",
                    self.identifier,
                    uri,
                    payload.len()
                );
                // trace!("Deserialize msg: {:?}", payload);
                if payload.len() == 0 {
                    debug!("Implement Ping!");
                } else {
                    self.priv_decode_on_receive(span, uri.clone(), payload.clone())?;
                }
            }
        };
        Ok(())
    }

    /// handle response we got from our parent
    #[allow(dead_code)]
    pub(crate) fn handle_transport_RequestToParentResponse(
        &mut self,
        _response: &transport::protocol::RequestToParentResponse,
    ) -> TransportResult<()> {
        // no-op
        Ok(())
    }
}
