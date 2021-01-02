use coerce::actor::context::ActorContext;
use coerce::actor::lifecycle::Status;
use coerce::actor::message::encoding::json::RemoteMessage;
use coerce::actor::message::{Handler, Message};
use coerce::actor::system::ActorSystem;
use coerce::actor::{new_actor, Actor};
use coerce::remote::net::StreamMessage;
use coerce::remote::stream::pubsub::{PubSub, StreamEvent, Topic};
use coerce::remote::system::RemoteActorSystem;
use tokio::time::Duration;

pub mod util;

#[macro_use]
extern crate serde;

#[macro_use]
extern crate async_trait;

#[derive(Debug)]
pub enum StatusEvent {
    Online,
    Offline,
}

pub struct StatusStream;

impl Topic for StatusStream {
    type Message = StatusEvent;

    fn topic_name() -> &'static str {
        "test-topic"
    }
}

pub struct TestStreamConsumer {
    received_stream_messages: i32,
}

#[async_trait]
impl Actor for TestStreamConsumer {
    async fn started(&mut self, ctx: &mut ActorContext) {
        PubSub::subscribe::<Self, StatusStream>(StatusStream, ctx)
            .await
            .unwrap()
    }

    async fn stopped(&mut self, ctx: &mut ActorContext) {
        PubSub::unsubscribe::<Self, StatusStream>(ctx)
            .await
            .unwrap()
    }
}

#[async_trait]
impl Handler<StreamEvent<StatusStream>> for TestStreamConsumer {
    async fn handle(&mut self, message: StreamEvent<StatusStream>, ctx: &mut ActorContext) {
        match message {
            StreamEvent::Receive(msg) => {
                log::info!("received msg: {:?}", &msg);

                self.received_stream_messages += 1;
            }
            StreamEvent::Err => {}
        }
    }
}

#[tokio::test]
pub async fn test_pubsub_local() {
    util::create_trace_logger();

    let mut sys = ActorSystem::new();
    let mut remote = RemoteActorSystem::builder()
        .with_actor_system(sys)
        .with_distributed_streams(|s| s.add_topic::<StatusStream>())
        .build()
        .await;

    let mut actor = remote
        .inner()
        .new_anon_actor(TestStreamConsumer {
            received_stream_messages: 0,
        })
        .await
        .unwrap();

    let mut actor_2 = remote
        .inner()
        .new_anon_actor(TestStreamConsumer {
            received_stream_messages: 0,
        })
        .await
        .unwrap();

    for _ in 0..10 {
        PubSub::publish(StatusStream, StatusEvent::Online, remote.inner()).await;
    }

    let received_stream_messages = actor.exec(|a| a.received_stream_messages).await.unwrap();
    let received_stream_messages_2 = actor_2.exec(|a| a.received_stream_messages).await.unwrap();

    assert_eq!(received_stream_messages, 10);
    assert_eq!(received_stream_messages_2, 10);
}


#[tokio::test]
pub async fn test_pubsub_distributed() {
    // util::create_trace_logger();

    let mut sys = ActorSystem::new();
    let mut remote = RemoteActorSystem::builder()
        .with_actor_system(sys)
        .with_distributed_streams(|s| s.add_topic::<StatusStream>())
        .build()
        .await;

    let mut sys = ActorSystem::new();
    let mut remote_b = RemoteActorSystem::builder()
        .with_actor_system(sys)
        .with_distributed_streams(|s| s.add_topic::<StatusStream>())
        .build()
        .await;

    remote.clone()
        .cluster_worker()
        .listen_addr("localhost:30101")
        .start()
        .await;

    remote_b.clone()
        .cluster_worker()
        .listen_addr("localhost:30102")
        .with_seed_addr("localhost:30101")
        .start()
        .await;

    let mut actor = remote
        .inner()
        .new_anon_actor(TestStreamConsumer {
            received_stream_messages: 0,
        })
        .await
        .unwrap();

    let mut actor_2 = remote_b
        .inner()
        .new_anon_actor(TestStreamConsumer {
            received_stream_messages: 0,
        })
        .await
        .unwrap();

    // Publish 5 messages on the first server
    for _ in 0..5 {
        PubSub::publish(StatusStream, StatusEvent::Online, remote.inner()).await;
    }

    // Publish 5 messages on the second server
    for _ in 0..5 {
        PubSub::publish(StatusStream, StatusEvent::Online, remote_b.inner()).await;
    }

    // remote stream publishing is asynchronous so there's no way to wait until actors have processed the messages
    // todo: create oneshot channel within test actors that publish once the actor receives 10 messages so we don't need to delay here
    tokio::time::sleep(Duration::from_millis(5)).await;

    // ensure both actors (one on each system) receives all stream messages from both servers
    let received_stream_messages = actor.exec(|a| a.received_stream_messages).await.unwrap();
    let received_stream_messages_2 = actor_2.exec(|a| a.received_stream_messages).await.unwrap();

    assert_eq!(received_stream_messages, 10);
    assert_eq!(received_stream_messages_2, 10);
}


impl StreamMessage for StatusEvent {
    fn read_from_bytes(data: Vec<u8>) -> Option<Self> {
        match data.first() {
            Some(0) => Some(StatusEvent::Offline),
            Some(1) => Some(StatusEvent::Online),
            _ => None,
        }
    }

    fn write_to_bytes(&self) -> Option<Vec<u8>> {
        match &self {
            StatusEvent::Offline => Some(vec![0]),
            StatusEvent::Online => Some(vec![1]),
        }
    }
}
