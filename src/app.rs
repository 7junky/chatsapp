use std::net::SocketAddr;
use std::sync::Arc;

use redis::Client as RedisClient;
use tokio::io::{self, AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::net::tcp::OwnedReadHalf;
use tokio::net::TcpStream;
use tokio::sync::mpsc::Sender;
use tokio::sync::Mutex;

use crate::broker::{self, BrokerEvent, RoomMap, SharedStream};
use crate::command::Command;
use crate::room::{self, RoomEvent};

pub struct User {
    addr: String,
    username: Option<String>,
}

enum State {
    Inside {
        room: String,
        tx: Sender<BrokerEvent>,
    },
    Outside,
}

pub struct App {
    redis: Arc<RedisClient>,
    stream: SharedStream,
    lines: Lines<BufReader<OwnedReadHalf>>,
    user: User,
    state: State,
}

impl App {
    pub fn new(stream: TcpStream, addr: SocketAddr, redis: Arc<RedisClient>) -> Self {
        let (reader, writer) = stream.into_split();
        let lines = BufReader::new(reader).lines();
        let stream = Arc::new(Mutex::new(writer));

        Self {
            redis,
            stream,
            lines,
            user: User {
                addr: addr.to_string(),
                username: None,
            },
            state: State::Outside,
        }
    }

    pub async fn run(mut self, room_map: RoomMap) -> io::Result<()> {
        self.write_greeting().await?;

        while let Some(message) = self.lines.next_line().await? {
            let command = Command::parse(message);
            let stream = self.stream.clone();

            match command {
                Command::Help => {
                    self.write_help().await?;
                }
                Command::List => {
                    match room::list(&self.redis).await {
                        Ok(list) => self.write_list(list, true).await?,
                        Err(e) => self.write_error(e).await?,
                    };
                }
                Command::Me => {
                    self.write_user_info().await?;
                }
                Command::SetUsername(username) => {
                    self.user.username = Some(username);
                }
                Command::CreateRoom(room) => {
                    if let Err(e) = room::new(&self.redis, &room).await {
                        self.write_error(e).await?
                    };

                    broker::spawn_broker(room, &room_map).await;
                }
                Command::JoinRoom(room) => {
                    if self.user.username.is_none() {
                        self.write_set_username().await?;
                        continue;
                    }

                    self.handle_join(Arc::clone(&stream), room.clone(), &room_map)
                        .await?;
                }
                Command::Message(msg) => {
                    self.handle_message(msg).await?;
                }
                Command::Leave => {
                    self.handle_leave().await?;
                }
                Command::Invalid => {
                    self.write_invalid().await?;
                }
                Command::Exit => break,
            }
        }

        Ok(())
    }

    async fn write_user_info(&self) -> io::Result<()> {
        let info = format!(
            "Username: {:?}, IP: {}\n",
            self.user.username, self.user.addr
        );

        self.write_all(info.as_bytes()).await?;

        Ok(())
    }

    async fn handle_message(&mut self, msg: String) -> io::Result<()> {
        match &self.state {
            State::Inside { room, tx } => self.send_message(tx, room, msg).await?,
            State::Outside => self.write_not_in_room().await?,
        }
        Ok(())
    }

    async fn handle_join(
        &mut self,
        stream: SharedStream,
        new_room: String,
        room_map: &RoomMap,
    ) -> io::Result<()> {
        match &self.state {
            State::Inside { room, tx } => {
                self.leave_room(tx, room).await?;

                if let Some(tx) = self.join_room(stream, room_map, &new_room).await? {
                    // Update state
                    self.state = State::Inside { room: new_room, tx }
                };
            }
            State::Outside => {
                if let Some(tx) = self.join_room(stream, &room_map, &new_room).await? {
                    // Update state
                    self.state = State::Inside { room: new_room, tx }
                }
            }
        }

        Ok(())
    }

    async fn handle_leave(&mut self) -> io::Result<()> {
        match &self.state {
            State::Inside { room, tx } => {
                self.leave_room(tx, room).await?;

                // Update state
                self.state = State::Outside
            }
            State::Outside => self.write_not_in_room().await?,
        }

        Ok(())
    }

    async fn send_message(
        &self,
        tx: &Sender<BrokerEvent>,
        room: &String,
        msg: String,
    ) -> io::Result<()> {
        let user = self.user.username.as_ref().unwrap();

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
                self.write_error(e).await?;
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
            self.write_error(e).await?;
        }

        Ok(())
    }

    async fn join_room(
        &self,
        stream: SharedStream,
        room_map: &RoomMap,
        room: &String,
    ) -> io::Result<Option<Sender<BrokerEvent>>> {
        let room_map = room_map.read().await;
        let user = self.user.username.as_ref().unwrap();

        // Get new rooms tx
        let tx = match room_map.get(room) {
            Some(tx) => tx.clone(),
            None => {
                self.write_room_not_found().await?;

                return Ok(None);
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
                self.write_error(e).await?;

                return Ok(None);
            }
        };

        // Send broker event
        if let Err(e) = tx
            .send(BrokerEvent::JoinRoom {
                user: user.to_owned(),
                stream: Arc::clone(&stream),
                msg: join_msg,
            })
            .await
        {
            self.write_error(e).await?;

            return Ok(None);
        };

        // Write recent messages
        let recent_msgs = match room::recent_msgs(&self.redis, &room).await {
            Ok(m) => m,
            Err(e) => {
                self.write_error(e).await?;

                // Connected by this point so return tx
                return Ok(Some(tx));
            }
        };
        self.write_list(recent_msgs, false).await?;

        Ok(Some(tx))
    }

    async fn leave_room(&self, tx: &Sender<BrokerEvent>, room: &String) -> io::Result<()> {
        let user = self.user.username.as_ref().unwrap();

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
                self.write_error(e).await?;
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
            self.write_error(e).await?;
        };

        Ok(())
    }

    async fn write_greeting(&self) -> io::Result<()> {
        let greeting = b"Welcome to ChatsApp!
Enter \">help\" for a list of commands and their usage.\n\n\n";

        self.write_all(greeting).await?;

        Ok(())
    }

    async fn write_invalid(&self) -> io::Result<()> {
        let invalid = b"Invalid command.
Enter \">help\" for a list of commands and their usage.\n";

        self.write_all(invalid).await?;

        Ok(())
    }

    async fn write_help(&self) -> io::Result<()> {
        let help = b"\
Commands:
>help              - Display commands
>exit              - Close connection
>list              - List rooms
>me                - Your user info
>set-username name - Set username
>create-room room  - Create room
>join-room room    - Join room\n";

        self.write_all(help).await?;

        Ok(())
    }

    async fn write_list(&self, list: Vec<String>, new_line: bool) -> io::Result<()> {
        let mut res = String::new();

        for item in list {
            res.push_str(&item);
            if new_line {
                res.push_str("\n");
            }
        }

        self.write_all(res.as_bytes()).await?;

        Ok(())
    }

    async fn write_error(&self, error: impl std::error::Error) -> io::Result<()> {
        self.write_all(error.to_string().as_bytes()).await?;

        Ok(())
    }

    async fn write_not_in_room(&self) -> io::Result<()> {
        self.write_all(b"You're not currently in a room.\n").await?;

        Ok(())
    }

    async fn write_room_not_found(&self) -> io::Result<()> {
        self.write_all(b"Room not found\n").await?;

        Ok(())
    }

    async fn write_set_username(&self) -> io::Result<()> {
        self.write_all(b"You need to pick a username before joining a room\n")
            .await?;

        Ok(())
    }

    async fn write_all(&self, bytes: &[u8]) -> io::Result<()> {
        let mut stream = self.stream.lock().await;
        stream.write_all(bytes).await?;

        Ok(())
    }
}
