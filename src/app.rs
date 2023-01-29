use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::Arc;

use redis::Client as RedisClient;
use tokio::io::{self, AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::Mutex;

use crate::broker::BrokerEvent;
use crate::room::RoomEvent;
use crate::{
    broker::{self, RoomMap, SharedStream},
    command::Command,
    room,
};

pub struct User {
    addr: String,
    username: Option<String>,
}

enum State {
    Inside(String),
    Outside,
}

pub struct App {
    redis: Arc<RedisClient>,
    user: User,
    state: State,
}

impl App {
    pub fn new(addr: SocketAddr, redis: Arc<RedisClient>) -> Self {
        Self {
            redis,
            user: User {
                addr: addr.to_string(),
                username: None,
            },
            state: State::Outside,
        }
    }

    pub async fn run(&mut self, stream: TcpStream, room_map: RoomMap) -> io::Result<()> {
        let (reader, writer) = stream.into_split();

        let buf_reader = BufReader::new(reader);
        let mut lines = buf_reader.lines();

        let stream = Arc::new(Mutex::new(writer));

        greeting(stream.clone()).await?;

        while let Some(message) = lines.next_line().await? {
            let command = Command::parse(message);
            let stream = stream.clone();

            match command {
                Command::Help => {
                    help(stream).await?;
                }
                Command::List => {
                    match room::list(&self.redis).await {
                        Ok(list) => rooms(stream, list).await?,
                        Err(e) => error(stream, e).await?,
                    };
                }
                Command::Me => {
                    self.user_info(stream).await?;
                }
                Command::SetUsername(username) => {
                    self.user.username = Some(username);
                }
                Command::CreateRoom(room) => {
                    if let Err(e) = room::new(&self.redis, &room).await {
                        error(stream, e).await?
                    };

                    broker::spawn_broker(room, &room_map).await;
                }
                Command::JoinRoom(room) => {
                    self.handle_join(stream, room, &room_map).await?;
                }
                Command::Message(msg) => {
                    self.handle_message(stream, msg, &room_map).await?;
                }
                Command::Leave => {
                    self.handle_leave(stream, &room_map).await?;
                }
                Command::Invalid => {
                    invalid(stream).await?;
                }
                Command::Exit => break,
            }
        }

        Ok(())
    }

    async fn user_info(&self, stream: SharedStream) -> io::Result<()> {
        let info = format!(
            "Username: {:?}, IP: {}\n",
            self.user.username, self.user.addr
        );

        write_all(stream, info.as_bytes()).await?;

        Ok(())
    }

    async fn handle_message(
        &mut self,
        stream: SharedStream,
        msg: String,
        room_map: &RoomMap,
    ) -> io::Result<()> {
        match self.state {
            State::Inside(ref room) => {
                // TODO: Check username not empty
                // TODO: Check redis errors
                let msg = room::event(
                    &self.redis,
                    RoomEvent::Chat(&msg),
                    "TODO",
                    self.user.username.as_ref().unwrap(),
                )
                .await
                .unwrap();

                let room_map = room_map.read().await;

                // Get room sender
                let tx = room_map.get(room).unwrap();

                // Send message event
                let user = self.user.username.as_ref().unwrap().to_owned();
                tx.send(BrokerEvent::Message { user, msg }).await;
            }
            State::Outside => not_in_room(stream).await?,
        }
        Ok(())
    }

    async fn handle_join(
        &mut self,
        stream: SharedStream,
        room: String,
        room_map: &RoomMap,
    ) -> io::Result<()> {
        let room_map = room_map.read().await;
        let user = self.user.username.as_ref().unwrap();

        match self.state {
            State::Inside(ref current_room) => {
                // TODO: Check username not empty
                // TODO: Check redis errors
                let leave_msg = room::event(
                    &self.redis,
                    RoomEvent::Leave,
                    &current_room,
                    self.user.username.as_ref().unwrap(),
                )
                .await
                .unwrap();

                // TODO: Check username not empty
                // TODO: Check redis errors
                let join_msg = room::event(
                    &self.redis,
                    RoomEvent::Join,
                    &room,
                    self.user.username.as_ref().unwrap(),
                )
                .await
                .unwrap();

                // Notify current room of leaving
                let current_tx = room_map.get(current_room).unwrap();

                current_tx
                    .send(BrokerEvent::LeaveRoom {
                        user: user.to_owned(),
                        msg: leave_msg,
                    })
                    .await;

                // Notify new room of joining
                let new_tx = room_map.get(&room).unwrap();
                new_tx
                    .send(BrokerEvent::JoinRoom {
                        user: user.to_owned(),
                        stream: Arc::clone(&stream),
                        msg: join_msg,
                    })
                    .await;

                // Update state
                self.state = State::Inside(room)
            }
            State::Outside => {
                // TODO: Check username not empty
                // TODO: Check no redis errors
                let msg = room::event(
                    &self.redis,
                    RoomEvent::Join,
                    &room,
                    self.user.username.as_ref().unwrap(),
                )
                .await
                .unwrap();

                // Notify new room of joining
                let new_tx = room_map.get(&room).unwrap();
                new_tx
                    .send(BrokerEvent::JoinRoom {
                        user: user.to_owned(),
                        stream: Arc::clone(&stream),
                        msg,
                    })
                    .await;

                // Update state
                self.state = State::Inside(room)
            }
        }

        Ok(())
    }

    async fn handle_leave(&mut self, stream: SharedStream, room_map: &RoomMap) -> io::Result<()> {
        let room_map = room_map.read().await;

        match self.state {
            State::Inside(ref room) => {
                let msg = room::event(
                    &self.redis,
                    RoomEvent::Leave,
                    room,
                    self.user.username.as_ref().unwrap(),
                )
                .await
                .unwrap();

                // Notify room of leaving
                let tx = room_map.get(room).unwrap();
                let user = self.user.username.as_ref().unwrap();

                tx.send(BrokerEvent::LeaveRoom {
                    user: user.to_owned(),
                    msg,
                })
                .await;

                // Update state
                self.state = State::Outside
            }
            State::Outside => not_in_room(stream).await?,
        }

        Ok(())
    }
}

async fn greeting(stream: SharedStream) -> io::Result<()> {
    let greeting = b"Welcome to ChatsApp!
Enter \">help\" for a list of commands and their usage.\n\n\n";

    write_all(stream, greeting).await?;

    Ok(())
}

async fn invalid(stream: SharedStream) -> io::Result<()> {
    let invalid = b"Invalid command.
Enter \">help\" for a list of commands and their usage.\n";

    write_all(stream, invalid).await?;

    Ok(())
}

async fn help(stream: SharedStream) -> io::Result<()> {
    let help = b"\
Commands:
>help              - Display commands
>exit              - Close connection
>list              - List rooms
>me                - Your user info
>set-username name - Set username
>create-room room  - Create room
>join-room room    - Join room\n";

    write_all(stream, help).await?;

    Ok(())
}

async fn rooms(stream: SharedStream, list: HashSet<String>) -> io::Result<()> {
    let mut res = String::new();

    for room in list {
        res.push_str(&room);
        res.push_str("\n");
    }

    write_all(stream, res.as_bytes()).await?;

    Ok(())
}

async fn error(stream: SharedStream, error: impl std::error::Error) -> io::Result<()> {
    write_all(stream, error.to_string().as_bytes()).await?;

    Ok(())
}

async fn not_in_room(stream: SharedStream) -> io::Result<()> {
    write_all(stream, b"You're not currently in a room.\n").await?;

    Ok(())
}

async fn write_all(stream: SharedStream, bytes: &[u8]) -> io::Result<()> {
    let mut stream = stream.lock().await;
    stream.write_all(bytes).await?;

    Ok(())
}
