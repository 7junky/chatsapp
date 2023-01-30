use std::{
    collections::{hash_map::Entry, HashMap},
    sync::Arc,
};

use tokio::{
    io::{self, AsyncWriteExt},
    net::tcp::OwnedWriteHalf,
    sync::{
        mpsc::{self, Receiver, Sender},
        Mutex, RwLock,
    },
};

pub type SharedStream = Arc<Mutex<OwnedWriteHalf>>;

pub enum BrokerEvent {
    JoinRoom {
        user: String,
        stream: SharedStream,
        msg: String,
    },
    LeaveRoom {
        user: String,
        msg: String,
    },
    Message {
        user: String,
        msg: String,
    },
}

pub type RoomMap = Arc<RwLock<HashMap<String, Sender<BrokerEvent>>>>;

pub fn bootstrap_rooms() -> RoomMap {
    // TODO: Since rooms are persisted in redis, calling this function
    // should fetch and store into map, spawning new brokers for each.

    Arc::new(RwLock::new(HashMap::new()))
}

pub async fn spawn_broker(room: String, rooms_map: &RoomMap) {
    let (room_tx, room_rx) = mpsc::channel(100);

    tokio::spawn(broker(room_rx));

    rooms_map.write().await.insert(room, room_tx);
}

pub async fn broker(mut events: Receiver<BrokerEvent>) -> io::Result<()> {
    // <User, Sender from the User>
    let mut users: HashMap<String, Sender<String>> = HashMap::new();

    while let Some(event) = events.recv().await {
        match event {
            BrokerEvent::JoinRoom { user, stream, msg } => {
                // Add user to peers:
                match users.entry(user) {
                    Entry::Occupied(..) => (),
                    Entry::Vacant(entry) => {
                        let (message_tx, message_rx) = mpsc::channel(100);
                        entry.insert(message_tx);
                        // This task is responsible for writing messages to the connected user.
                        tokio::spawn(receive_messages(message_rx, stream));
                    }
                };
                // TODO: write message to users
            }
            BrokerEvent::LeaveRoom { user, msg } => {
                // Remove user from peers:
                users.remove(&user);
                // TODO: write message to users
            }
            BrokerEvent::Message { user, msg } => {
                // Loop over each user in the room
                for (u, sender) in &users {
                    // If they're the sender of the message, skip since they'll see
                    // their message twice
                    if u == &user {
                        continue;
                    }

                    // Send to each user
                    if let Err(e) = sender.send(msg.clone()).await {
                        eprintln!("{}", e);
                    };
                }
            }
        }
    }

    Ok(())
}

async fn receive_messages(mut messages: Receiver<String>, stream: SharedStream) {
    // Dropping the Sender should kill this task
    while let Some(msg) = messages.recv().await {
        let mut stream = stream.lock().await;

        if let Err(e) = stream.write_all(msg.as_bytes()).await {
            eprintln!("{}", e);
        };
    }
}
