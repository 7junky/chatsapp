use std::{
    collections::{hash_map::Entry, HashMap},
    sync::Arc,
};

use tokio::{
    io::{self, AsyncWrite, AsyncWriteExt},
    sync::{
        mpsc::{self, Receiver, Sender},
        Mutex,
    },
};

pub enum Event<T> {
    JoinRoom {
        user: String,
        stream: Arc<Mutex<T>>,
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

// Shouldn't this be per room?
async fn broker<T>(mut events: Receiver<Event<T>>) -> io::Result<()>
where
    T: AsyncWrite + Unpin + Send + 'static,
{
    // <User, Sender from the User>
    let mut users: HashMap<String, Sender<String>> = HashMap::new();

    while let Some(event) = events.recv().await {
        match event {
            Event::JoinRoom { user, stream, msg } => {
                // Add user to peers:
                match users.entry(user) {
                    Entry::Occupied(..) => (),
                    Entry::Vacant(entry) => {
                        let (client_sender, client_receiver) = mpsc::channel(100);
                        entry.insert(client_sender);
                        // This task is responsible for writing messages to the connected user.
                        tokio::spawn(receive_messages(client_receiver, stream));
                    }
                };
                // TODO: write message to users
            }
            Event::LeaveRoom { user, msg } => {
                // Remove user from peers:
                users.remove(&user);
                // TODO: write message to users
            }
            Event::Message { user, msg } => {
                // Loop over each user in the room
                for (u, sender) in &users {
                    // If they're the sender of the message, skip since they'll see
                    // their message twice
                    if u == &user {
                        break;
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

// async fn receive_messages<T>(mut messages: Receiver<String>, stream: Arc<Mutex<T>>)
async fn receive_messages<T>(mut messages: Receiver<String>, stream: Arc<Mutex<T>>)
where
    T: AsyncWrite + Unpin,
{
    // Dropping the Sender should kill this task
    while let Some(msg) = messages.recv().await {
        let mut stream = stream.lock().await;

        if let Err(e) = stream.write_all(msg.as_bytes()).await {
            eprintln!("{}", e);
        };
    }
}
