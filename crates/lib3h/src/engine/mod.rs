pub mod engine_actor;
pub mod ghost_engine;
pub mod ghost_engine_wrapper;
mod network_layer;
pub mod p2p_protocol;
mod space_layer;

use crate::{
    dht::dht_protocol::*,
    error::*,
    gateway::{protocol::*, P2pGateway},
    track::Tracker,
    transport::{websocket::tls::TlsConfig, TransportMultiplex},
};
use detach::Detach;
use lib3h_crypto_api::{Buffer, CryptoSystem};
use lib3h_ghost_actor::prelude::*;
use lib3h_protocol::{protocol::*, Address};
use serde::{ser::SerializeSeq, Deserialize, Deserializer, Serializer};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
};
use url::Url;

/// Identifier of a source chain: SpaceAddress+AgentId
pub type ChainId = (Address, Address);

pub static NETWORK_GATEWAY_ID: &'static str = "__network__";

fn vec_url_de<'de, D>(deserializer: D) -> Result<Vec<Url>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    struct Wrapper(#[serde(with = "url_serde")] Url);

    let v = Vec::deserialize(deserializer)?;
    Ok(v.into_iter().map(|Wrapper(a)| a).collect())
}

fn vec_url_se<S>(v: &[Url], serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    #[derive(Serialize)]
    struct Wrapper(#[serde(with = "url_serde")] Url);

    let mut seq = serializer.serialize_seq(Some(v.len()))?;
    for u in v {
        seq.serialize_element(&Wrapper(u.clone()))?;
    }
    seq.end()
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
enum RealEngineTrackerData {
    /// track the actual HandleGetGossipingEntryList request
    GetGossipingEntryList,
    /// track the actual HandleGetAuthoringEntryList request
    GetAuthoringEntryList,
    /// once we have the AuthoringEntryListResponse, fetch data for entries
    DataForAuthorEntry,
    /// gossip has requested we store data, send a hold request to core
    /// core should respond ??
    HoldEntryRequested,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
/// Transport specific configuration
pub enum TransportConfig {
    Websocket(TlsConfig),
    Memory(String),
}

/// Struct holding all config settings for the Engine
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct EngineConfig {
    pub network_id: Address,
    pub transport_configs: Vec<TransportConfig>,
    #[serde(deserialize_with = "vec_url_de", serialize_with = "vec_url_se")]
    pub bootstrap_nodes: Vec<Url>,
    pub work_dir: PathBuf,
    pub log_level: char,
    #[serde(with = "url_serde")]
    pub bind_url: Url,
    pub dht_gossip_interval: u64,
    pub dht_timeout_threshold: u64,
    pub dht_custom_config: Vec<u8>,
}

pub struct TransportKeys {
    /// Our TransportId, i.e. Base32 encoded public key (e.g. "HcMyadayada")
    pub transport_id: String,
    /// The TransportId public key
    pub transport_public_key: Box<dyn Buffer>,
    /// The TransportId secret key
    pub transport_secret_key: Box<dyn Buffer>,
}
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

pub trait CanAdvertise {
    fn advertise(&self) -> Url;
}

pub struct GhostEngine<'engine> {
    /// Identifier
    name: String,
    /// Config settings
    config: EngineConfig,
    /// Factory for building the DHTs used by the gateways
    dht_factory: DhtFactory,
    /// Tracking request_id's sent to core
    request_track: Tracker<RealEngineTrackerData>,
    /// Multiplexer holding the network gateway
    multiplexer: Detach<GatewayParentWrapper<GhostEngine<'engine>, TransportMultiplex<P2pGateway>>>,
    /// Cached this_peer of the multiplexer
    this_net_peer: PeerData,

    /// Store active connections?
    network_connections: HashSet<Url>,
    /// Map of P2p gateway per Space+Agent
    space_gateway_map:
        HashMap<ChainId, Detach<GatewayParentWrapper<GhostEngine<'engine>, P2pGateway>>>,
    #[allow(dead_code)]
    /// crypto system to use
    crypto: Box<dyn CryptoSystem>,
    #[allow(dead_code)]
    /// transport_id data, public/private keys, etc
    transport_keys: TransportKeys,

    client_endpoint: Option<
        GhostEndpoint<
            ClientToLib3h,
            ClientToLib3hResponse,
            Lib3hToClient,
            Lib3hToClientResponse,
            Lib3hError,
        >,
    >,
    lib3h_endpoint: Detach<
        GhostContextEndpoint<
            GhostEngine<'engine>,
            Lib3hToClient,
            Lib3hToClientResponse,
            ClientToLib3h,
            ClientToLib3hResponse,
            Lib3hError,
        >,
    >,
}
