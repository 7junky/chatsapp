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

#[derive(Debug)]
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
    // <User, Sender for the User>
    let mut users: HashMap<String, Sender<String>> = HashMap::new();

    while let Some(event) = events.recv().await {
        match event {
            BrokerEvent::JoinRoom { user, stream, msg } => {
                // Add user to peers:
                match users.entry(user.clone()) {
                    Entry::Occupied(..) => (),
                    Entry::Vacant(entry) => {
                        // Each user will have a tx associated with their name and
                        // an rx associated with their tcp connection
                        let (message_tx, message_rx) = mpsc::channel(100);
                        entry.insert(message_tx);

                        // This task is responsible for writing messages to the connected user.
                        tokio::spawn(receive_messages(message_rx, stream));

                        // Send join msg:
                        send_messages(msg, user, &users).await;
                    }
                };
            }
            BrokerEvent::LeaveRoom { user, msg } => {
                // Remove user from peers:
                users.remove(&user);

                // Send leave msg
                send_messages(msg, user, &users).await;
            }
            BrokerEvent::Message { user, msg } => {
                send_messages(msg, user, &users).await;
            }
        }
    }

    Ok(())
}

async fn send_messages(msg: String, sender: String, users: &HashMap<String, Sender<String>>) {
    // Loop over each user in the room
    for (user, tx) in users {
        // If they're the sender of the message, skip since they'll see
        // their message twice
        if user == &sender {
            continue;
        }

        // Send to each user
        if let Err(e) = tx.send(msg.clone()).await {
            eprintln!("{}", e);
        };
    }
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
