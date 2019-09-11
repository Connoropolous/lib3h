//! SimChat is a simulation test framework backend for lib3h
//! - See the integration tests for automated usage
//! - See half_busy_chat_cli for manual simulation

extern crate base64;
extern crate crossbeam_channel;
extern crate url;
#[cfg(test)]
#[macro_use]
extern crate detach;

pub mod simchat;
pub use simchat::{ChatEvent, SimChat, SimChatMessage};

use lib3h::error::Lib3hError;
use lib3h_crypto_api::CryptoError;
use lib3h_ghost_actor::{
    GhostActor, GhostCallbackData::Response, GhostCanTrack, GhostContextEndpoint,
};
use lib3h_protocol::{
    data_types::{ConnectData, DirectMessageData, SpaceData, QueryEntryData, QueryEntryResultData},
    protocol::{ClientToLib3h, ClientToLib3hResponse, Lib3hToClient, Lib3hToClientResponse},
    Address,
};
use lib3h_sodium::{hash, secbuf::SecBuf};

use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use std::time::{SystemTime, UNIX_EPOCH};
use url::Url;

type EngineBuilder<T> = fn() -> T;

pub type HandleEvent = Box<dyn FnMut(&ChatEvent) + Send>;

pub struct Lib3hSimChat {
    thread: Option<std::thread::JoinHandle<()>>,
    thread_continue: Arc<AtomicBool>,
    out_send: crossbeam_channel::Sender<ChatEvent>,
}

impl Drop for Lib3hSimChat {
    fn drop(&mut self) {
        self.thread_continue.store(false, Ordering::Relaxed);
        std::mem::replace(&mut self.thread, None)
            .expect("cannot drop, thread already None")
            .join()
            .expect("thead join failed");
    }
}

impl Lib3hSimChat {
    pub fn new<T>(engine_builder: EngineBuilder<T>, mut handler: HandleEvent, peer_uri: Url) -> Self
    where
        T: GhostActor<
                Lib3hToClient,
                Lib3hToClientResponse,
                ClientToLib3h,
                ClientToLib3hResponse,
                Lib3hError,
            > + 'static,
    {
        let thread_continue = Arc::new(AtomicBool::new(true));

        let (out_send, out_recv): (
            crossbeam_channel::Sender<_>,
            crossbeam_channel::Receiver<ChatEvent>,
        ) = crossbeam_channel::unbounded();

        let internal_sender = out_send.clone();

        let thread_continue_inner = thread_continue.clone();
        Self {
            thread: Some(std::thread::spawn(move || {
                // this thread owns the ghost engine instance
                // and is responsible for calling process
                // and handling messages
                let mut engine = engine_builder();
                let mut parent_endpoint: GhostContextEndpoint<(), String, _, _, _, _, _> = engine
                    .take_parent_endpoint()
                    .unwrap()
                    .as_context_endpoint_builder()
                    .request_id_prefix("parent")
                    .build();

                // also keep track of things like the spaces and current space in this scope
                let mut current_space: Option<SpaceData> = None;
                let mut spaces: HashMap<Address, SpaceData> = HashMap::new();
                let _cas: HashMap<Address, HashMap<Address, SimChatMessage>> = HashMap::new();

                // call connect to start the networking process
                // (should probably wait for confirmatio before continuing)
                Self::connect(&mut parent_endpoint, peer_uri);

                while thread_continue_inner.load(Ordering::Relaxed) {
                    // call process to make stuff happen
                    parent_endpoint.process(&mut ()).unwrap();
                    engine.process().unwrap();

                    // grab any new events from lib3h
                    let engine_chat_events = parent_endpoint
                        .drain_messages();

                    // gather all the ChatEvents
                    // Receive directly from the crossbeam channel
                    // and convert relevent N3H protocol messages to chat events
                    let direct_chat_events = out_recv.try_iter();
                    let engine_chat_events = engine_chat_events
                        .into_iter()
                        // process any that should be handled silently (without generating a chat event)
                        .filter_map(|mut engine_message| {
                            match engine_message.take_message() {
                                Some(Lib3hToClient::HandleQueryEntry(QueryEntryData {
                                    space_address,
                                    entry_address,
                                    request_id,
                                    requester_agent_id,
                                    query: _,
                                })) => {
                                    engine_message.respond(Ok(Lib3hToClientResponse::HandleQueryEntryResult(QueryEntryResultData {
                                        request_id,
                                        entry_address,
                                        requester_agent_id: requester_agent_id.clone(),
                                        space_address: space_address.clone(),
                                        responder_agent_id: requester_agent_id,
                                        query_result: Vec::new(),
                                    }))).ok();
                                    None
                                },
                                Some(Lib3hToClient::HandleStoreEntryAspect{..}) => {
                                    None
                                },
                                Some(Lib3hToClient::HandleGetAuthoringEntryList{..}) => {
                                    None
                                },
                                Some(Lib3hToClient::HandleGetGossipingEntryList{..}) => {
                                    None
                                },
                                Some(Lib3hToClient::HandleSendDirectMessage(message_data)) => {
                                    Some(ChatEvent::ReceiveDirectMessage(SimChatMessage {
                                        from_agent: message_data.from_agent_id.to_string(),
                                        payload: "message from engine".to_string(),
                                        timestamp: current_timestamp(),
                                    }))
                                }
                                Some(Lib3hToClient::Disconnected(_)) => Some(ChatEvent::Disconnected),
                                Some(_) => {None}, // event we don't care about
                                None => {None}, // there was nothing in the message
                            }
                        });

                    // process all the chat events by calling the handler for all events
                    // and dispatching new n3h actions where required
                    for chat_event in direct_chat_events.chain(engine_chat_events) {
                        let local_internal_sender = internal_sender.clone();

                        // every chat event call the handler that was passed
                        handler(&chat_event);

                        // also do internal logic for certain events e.g. converting them to lib3h events
                        // and also handling the responses to mutate local state
                        match chat_event {
                            ChatEvent::Join {
                                channel_id,
                                agent_id,
                            } => {
                                let space_address =
                                    channel_address_from_string(&channel_id).unwrap();
                                let space_data = SpaceData {
                                    agent_id: Address::from(agent_id),
                                    request_id: "".to_string(),
                                    space_address,
                                };
                                parent_endpoint
                                    .request(
                                        String::from("ctx"),
                                        ClientToLib3h::JoinSpace(space_data.clone()),
                                        Box::new(move |_, _, callback_data| {
                                            println!(
                                                "chat received response from engine: {:?}",
                                                callback_data
                                            );
                                            if let Response(Ok(_payload)) = callback_data {
                                                local_internal_sender
                                                    .send(ChatEvent::JoinSuccess {
                                                        channel_id: channel_id.clone(),
                                                        space_data: space_data.clone(),
                                                    })
                                                    .unwrap();
                                            }
                                            Ok(())
                                        }),
                                    )
                                    .unwrap();
                            }

                            ChatEvent::JoinSuccess {
                                space_data,
                                channel_id,
                            } => {
                                spaces.insert(channel_address_from_string(&channel_id).unwrap(), space_data.clone());
                                current_space = Some(space_data);
                                Self::send_sys_message(
                                    local_internal_sender,
                                    &format!("Joined channel: {}", channel_id),
                                );
                            }

                            ChatEvent::Part(channel_id) => {
                                if let Some(space_data) = current_space.clone() {
                                    parent_endpoint
                                        .request(
                                            String::from("ctx"),
                                            ClientToLib3h::LeaveSpace(space_data.to_owned()),
                                            Box::new(move |_, _, callback_data| {
                                                println!(
                                                    "chat received response from engine: {:?}",
                                                    callback_data
                                                );
                                                if let Response(Ok(_payload)) = callback_data {
                                                    local_internal_sender
                                                        .send(ChatEvent::PartSuccess(
                                                            channel_id.clone(),
                                                        ))
                                                        .unwrap();
                                                }
                                                Ok(())
                                            }),
                                        )
                                        .unwrap();
                                } else {
                                    Self::send_sys_message(
                                        local_internal_sender,
                                        &"No channel to leave".to_string(),
                                    );
                                }
                            }

                            ChatEvent::PartSuccess(channel_id) => {
                                current_space = None;
                                spaces.remove(&channel_address_from_string(&channel_id).unwrap());
                                Self::send_sys_message(
                                    local_internal_sender,
                                    &"Left channel".to_string(),
                                );
                            }

                            ChatEvent::SendDirectMessage { to_agent, payload } => {
                                if let Some(space_data) = current_space.clone() {
                                    let direct_message_data = DirectMessageData {
                                        from_agent_id: space_data.agent_id,
                                        content: payload.into(),
                                        to_agent_id: Address::from(to_agent),
                                        space_address: space_data.space_address,
                                        request_id: String::from(""),
                                    };
                                    parent_endpoint
                                        .request(
                                            String::from("ctx"),
                                            ClientToLib3h::SendDirectMessage(direct_message_data),
                                            Box::new(|_, _, callback_data| {
                                                println!(
                                                    "chat received response from engine: {:?}",
                                                    callback_data
                                                );
                                                // TODO: Track delivered state of message
                                                Ok(())
                                            }),
                                        )
                                        .unwrap();
                                } else {
                                    Self::send_sys_message(
                                        local_internal_sender,
                                        &"Must join a channel before sending a message".to_string(),
                                    );
                                }
                            },

                            ChatEvent::SendChannelMessage{..} => {

                            }

                            _ => {}
                        }
                    }

                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
            })),
            thread_continue,
            out_send,
        }
    }

    fn send_sys_message(sender: crossbeam_channel::Sender<ChatEvent>, msg: &String) {
        sender
            .send(ChatEvent::ReceiveDirectMessage(SimChatMessage{
                from_agent: String::from("sys"),
                payload: String::from(msg),
                timestamp: current_timestamp(),
            }))
            .expect("send fail");
    }

    fn connect(
        endpoint: &mut GhostContextEndpoint<
            (),
            String,
            ClientToLib3h,
            ClientToLib3hResponse,
            Lib3hToClient,
            Lib3hToClientResponse,
            Lib3hError,
        >,
        peer_uri: Url,
    ) {
        let connect_message = ClientToLib3h::Connect(ConnectData {
            network_id: String::from(""), // connect to any
            peer_uri,
            request_id: String::from("connect-request"),
        });
        endpoint
            .request(
                String::from("ctx"),
                connect_message,
                Box::new(|_, _, callback_data| {
                    println!("chat received response from engine: {:?}", callback_data);
                    Ok(())
                }),
            )
            .unwrap();
    }
}

impl SimChat for Lib3hSimChat {
    fn send(&mut self, event: ChatEvent) {
        self.out_send.send(event).expect("send fail");
    }
}

pub fn channel_address_from_string(channel_id: &String) -> Result<Address, CryptoError> {
    let mut input = SecBuf::with_insecure_from_string(channel_id.to_string());
    let mut output = SecBuf::with_insecure(hash::BYTES256);
    hash::sha256(&mut input, &mut output).unwrap();
    let buf = output.read_lock();
    let signature_str = base64::encode(&**buf);
    Ok(Address::from(signature_str))
}

pub fn current_timestamp() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    mod mock_engine;
    use mock_engine::MockEngine;

    fn new_sim_chat_mock_engine(callback: HandleEvent) -> Lib3hSimChat {
        Lib3hSimChat::new(
            MockEngine::new,
            callback,
            Url::parse("http://test.boostrap").unwrap(),
        )
    }

    /*----------  example messages  ----------*/

    fn chat_event() -> ChatEvent {
        ChatEvent::SendDirectMessage {
            to_agent: "addr".to_string(),
            payload: "yo".to_string(),
        }
    }

    fn receive_sys_message(payload: String) -> ChatEvent {
        ChatEvent::ReceiveDirectMessage(SimChatMessage{
            from_agent: String::from("sys"),
            payload,
            timestamp: current_timestamp(),
        })
    }

    fn join_event() -> ChatEvent {
        ChatEvent::Join {
            agent_id: "test_agent".to_string(),
            channel_id: "test_channel".to_string(),
        }
    }

    fn join_success_event() -> ChatEvent {
        let space_address = channel_address_from_string(&"test_channel".to_string())
            .expect("failed to hash string");

        ChatEvent::JoinSuccess {
            channel_id: "test_channel".to_string(),
            space_data: SpaceData {
                agent_id: Address::from("test_agent"),
                request_id: "".to_string(),
                space_address,
            },
        }
    }

    fn part_event() -> ChatEvent {
        ChatEvent::Part("test_channel".to_string())
    }

    fn part_success_event() -> ChatEvent {
        ChatEvent::PartSuccess("test_channel".to_string())
    }

    #[test]
    fn it_should_echo() {
        let (s, r) = crossbeam_channel::unbounded();
        let mut chat = new_sim_chat_mock_engine(Box::new(move |event| {
            s.send(event.to_owned()).expect("send fail");
        }));
        chat.send(chat_event());
        assert_eq!(chat_event(), r.recv().expect("receive fail"));
    }

    /*----------  join/part ----------*/

    #[test]
    fn calling_join_space_triggers_success_message() {
        let (s, r) = crossbeam_channel::unbounded();
        let mut chat = new_sim_chat_mock_engine(Box::new(move |event| {
            s.send(event.to_owned()).expect("send fail");
        }));

        chat.send(join_event());

        let chat_messages = r.iter().take(2).collect::<Vec<_>>();
        assert_eq!(chat_messages, vec![join_event(), join_success_event(),],);
    }

    #[test]
    fn calling_part_before_join_fails() {
        let (s, r) = crossbeam_channel::unbounded();
        let mut chat = new_sim_chat_mock_engine(Box::new(move |event| {
            s.send(event.to_owned()).expect("send fail");
        }));

        chat.send(part_event());

        let chat_messages = r.iter().take(2).collect::<Vec<_>>();
        assert_eq!(
            chat_messages,
            vec![
                part_event(),
                receive_sys_message("No channel to leave".to_string()),
            ],
        );
    }

    #[test]
    fn calling_join_then_part_succeeds() {
        let (s, r) = crossbeam_channel::unbounded();
        let mut chat = new_sim_chat_mock_engine(Box::new(move |event| {
            s.send(event.to_owned()).expect("send fail");
        }));

        chat.send(join_event());
        std::thread::sleep(std::time::Duration::from_millis(100)); // find a better way
        chat.send(part_event());

        let chat_messages = r.iter().take(5).collect::<Vec<_>>();
        assert_eq!(
            chat_messages,
            vec![
                join_event(),
                join_success_event(),
                receive_sys_message("Joined channel: test_channel".to_string()),
                part_event(),
                part_success_event(),
            ],
        );
    }

    /*----------  direct message  ----------*/

    #[test]
    fn sending_direct_message_before_join_fails() {
        let (s, r) = crossbeam_channel::unbounded();
        let mut chat = new_sim_chat_mock_engine(Box::new(move |event| {
            s.send(event.to_owned()).expect("send fail");
        }));

        chat.send(chat_event());

        let chat_messages = r.iter().take(2).collect::<Vec<_>>();
        assert_eq!(
            chat_messages,
            vec![
                chat_event(),
                receive_sys_message("Must join a channel before sending a message".to_string()),
            ],
        );
    }

    #[test]
    fn can_join_and_send_direct_message() {
        let (s, r) = crossbeam_channel::unbounded();
        let mut chat = new_sim_chat_mock_engine(Box::new(move |event| {
            s.send(event.to_owned()).expect("send fail");
        }));

        chat.send(join_event());
        std::thread::sleep(std::time::Duration::from_millis(100)); // find a better way
        chat.send(chat_event());

        let chat_messages = r.iter().take(4).collect::<Vec<_>>();
        assert_eq!(
            chat_messages,
            vec![
                join_event(),
                join_success_event(),
                receive_sys_message("Joined channel: test_channel".to_string()),
                chat_event(),
            ],
        );
    }

    #[test]
    fn can_convert_strings_to_channel_address() {
        let addr = channel_address_from_string(&String::from("test channel id"))
            .expect("failed to hash string");
        assert_eq!(
            addr,
            Address::from("mgb/+MzdPWAYRs4ERGlj3WCg53AddXsg1HjXH7pk7pE=".to_string())
        )
    }
}
