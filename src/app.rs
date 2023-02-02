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

        write_greeting(stream.clone()).await?;

        while let Some(message) = lines.next_line().await? {
            let command = Command::parse(message);
            let stream = stream.clone();

            match command {
                Command::Help => {
                    write_help(stream).await?;
                }
                Command::List => {
                    match room::list(&self.redis).await {
                        Ok(list) => write_list(stream, list, true).await?,
                        Err(e) => write_error(stream, e).await?,
                    };
                }
                Command::Me => {
                    self.write_user_info(stream).await?;
                }
                Command::SetUsername(username) => {
                    self.user.username = Some(username);
                }
                Command::CreateRoom(room) => {
                    if let Err(e) = room::new(&self.redis, &room).await {
                        write_error(stream, e).await?
                    };

                    broker::spawn_broker(room, &room_map).await;
                }
                Command::JoinRoom(room) => {
                    if self.user.username.is_none() {
                        write_set_username(stream).await?;
                        continue;
                    }

                    self.handle_join(Arc::clone(&stream), room.clone(), &room_map)
                        .await?;
                }
                Command::Message(msg) => {
                    self.handle_message(stream, msg, &room_map).await?;
                }
                Command::Leave => {
                    self.handle_leave(stream, &room_map).await?;
                }
                Command::Invalid => {
                    write_invalid(stream).await?;
                }
                Command::Exit => break,
            }
        }

        Ok(())
    }

    async fn write_user_info(&self, stream: SharedStream) -> io::Result<()> {
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
            State::Inside(ref room) => self.send_message(stream, &room_map, room, msg).await?,
            State::Outside => write_not_in_room(stream).await?,
        }
        Ok(())
    }

    async fn handle_join(
        &mut self,
        stream: SharedStream,
        new_room: String,
        room_map: &RoomMap,
    ) -> io::Result<()> {
        match self.state {
            State::Inside(ref current_room) => {
                self.leave_room(stream.clone(), &room_map, current_room)
                    .await?;

                self.join_room(stream, room_map, &new_room).await?;

                // Update state
                self.state = State::Inside(new_room)
            }
            State::Outside => {
                self.join_room(stream, &room_map, &new_room).await?;

                // Update state
                self.state = State::Inside(new_room)
            }
        }

        Ok(())
    }

    async fn handle_leave(&mut self, stream: SharedStream, room_map: &RoomMap) -> io::Result<()> {
        match self.state {
            State::Inside(ref room) => {
                self.leave_room(stream, &room_map, room).await?;

                // Update state
                self.state = State::Outside
            }
            State::Outside => write_not_in_room(stream).await?,
        }

        Ok(())
    }

    async fn send_message(
        &self,
        stream: SharedStream,
        room_map: &RoomMap,
        room: &String,
        msg: String,
    ) -> io::Result<()> {
        let room_map = room_map.read().await;
        let user = self.user.username.as_ref().unwrap();

        // Get room tx
        let tx = room_map.get(room).unwrap();

        let msg = match room::event(
            &self.redis,
            RoomEvent::Chat(msg),
            room,
            self.user.username.as_ref().unwrap(),
        )
        .await
        {
            Ok(msg) => msg,
            Err(e) => {
                write_error(stream, e).await?;
                return Ok(());
            }
        };

        // Send broker event
        if let Err(e) = tx
            .send(BrokerEvent::Message {
                user: user.to_owned(),
                msg,
            })
            .await
        {
            write_error(stream, e).await?;
        }

        Ok(())
    }

    async fn join_room(
        &self,
        stream: SharedStream,
        room_map: &RoomMap,
        room: &String,
    ) -> io::Result<()> {
        let room_map = room_map.read().await;
        let user = self.user.username.as_ref().unwrap();

        // Get new rooms tx
        let new_tx = match room_map.get(room) {
            Some(tx) => tx,
            None => {
                write_room_not_found(stream.clone()).await?;

                return Ok(());
            }
        };

        // Join message
        let join_msg = match room::event(
            &self.redis,
            RoomEvent::Join,
            &room,
            self.user.username.as_ref().unwrap(),
        )
        .await
        {
            Ok(msg) => msg,
            Err(e) => {
                write_error(stream, e).await?;

                return Ok(());
            }
        };

        // Send broker event
        if let Err(e) = new_tx
            .send(BrokerEvent::JoinRoom {
                user: user.to_owned(),
                stream: Arc::clone(&stream),
                msg: join_msg,
            })
            .await
        {
            write_error(stream.clone(), e).await?;
        };

        // Write recent messages
        let recent_msgs = match room::recent_msgs(&self.redis, &room).await {
            Ok(m) => m,
            Err(e) => {
                write_error(stream, e).await?;

                return Ok(());
            }
        };
        write_list(stream, recent_msgs, false).await?;

        Ok(())
    }

    async fn leave_room(
        &self,
        stream: SharedStream,
        room_map: &RoomMap,
        room: &String,
    ) -> io::Result<()> {
        let room_map = room_map.read().await;
        let user = self.user.username.as_ref().unwrap();

        // Get rooms tx
        let tx = room_map.get(room).unwrap();

        // Leave msg
        let msg = match room::event(
            &self.redis,
            RoomEvent::Leave,
            room,
            self.user.username.as_ref().unwrap(),
        )
        .await
        {
            Ok(msg) => msg,
            Err(e) => {
                write_error(stream, e).await?;
                return Ok(());
            }
        };

        // Send broker event
        if let Err(e) = tx
            .send(BrokerEvent::LeaveRoom {
                user: user.to_owned(),
                msg,
            })
            .await
        {
            write_error(stream, e).await?;
        };

        Ok(())
    }
}

async fn write_greeting(stream: SharedStream) -> io::Result<()> {
    let greeting = b"Welcome to ChatsApp!
Enter \">help\" for a list of commands and their usage.\n\n\n";

    write_all(stream, greeting).await?;

    Ok(())
}

async fn write_invalid(stream: SharedStream) -> io::Result<()> {
    let invalid = b"Invalid command.
Enter \">help\" for a list of commands and their usage.\n";

    write_all(stream, invalid).await?;

    Ok(())
}

async fn write_help(stream: SharedStream) -> io::Result<()> {
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

async fn write_list(stream: SharedStream, list: Vec<String>, new_line: bool) -> io::Result<()> {
    let mut res = String::new();

    for item in list {
        res.push_str(&item);
        if new_line {
            res.push_str("\n");
        }
    }

    write_all(stream, res.as_bytes()).await?;

    Ok(())
}

async fn write_error(stream: SharedStream, error: impl std::error::Error) -> io::Result<()> {
    write_all(stream, error.to_string().as_bytes()).await?;

    Ok(())
}

async fn write_not_in_room(stream: SharedStream) -> io::Result<()> {
    write_all(stream, b"You're not currently in a room.\n").await?;

    Ok(())
}

async fn write_room_not_found(stream: SharedStream) -> io::Result<()> {
    write_all(stream, b"Room not found\n").await?;

    Ok(())
}

async fn write_set_username(stream: SharedStream) -> io::Result<()> {
    write_all(
        stream,
        b"You need to pick a username before joining a room\n",
    )
    .await?;

    Ok(())
}

async fn write_all(stream: SharedStream, bytes: &[u8]) -> io::Result<()> {
    let mut stream = stream.lock().await;
    stream.write_all(bytes).await?;

    Ok(())
}
