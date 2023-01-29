use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::Arc;

use redis::Client as RedisClient;
use tokio::io::{self, AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

use crate::command::Command;
use crate::room;

struct User {
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

    pub async fn run(&mut self, mut stream: TcpStream) -> io::Result<()> {
        greeting(&mut stream).await?;

        loop {
            let reader = BufReader::new(&mut stream);

            let message = reader
                .lines()
                .next_line()
                .await
                .map(|v| v.unwrap_or_default())?;

            let command = Command::parse(message);

            match command {
                Command::Help => {
                    help(&mut stream).await?;
                }
                Command::List => {
                    match room::list(&self.redis).await {
                        Ok(list) => rooms(&mut stream, list).await?,
                        Err(e) => error(&mut stream, e).await?,
                    };
                }
                Command::Me => {
                    self.user_info(&mut stream).await?;
                }
                Command::SetUsername(username) => {
                    self.user.username = Some(username);
                }
                Command::CreateRoom(room) => {
                    if let Err(e) = room::new(&self.redis, &room).await {
                        error(&mut stream, e).await?
                    };
                }
                Command::JoinRoom(room) => {
                    self.handle_join(&mut stream, room).await?;
                }
                Command::Message(msg) => {
                    self.handle_message(&mut stream, msg).await?;
                }
                Command::Leave => {
                    self.handle_leave(&mut stream).await?;
                }
                Command::Invalid => {
                    invalid(&mut stream).await?;
                }
                Command::Exit => break,
            }
        }

        Ok(())
    }

    async fn user_info(&self, stream: &mut TcpStream) -> io::Result<()> {
        let info = format!(
            "Username: {:?}, IP: {}\n",
            self.user.username, self.user.addr
        );

        stream.write_all(info.as_bytes()).await?;

        Ok(())
    }

    async fn handle_message(&mut self, stream: &mut TcpStream, msg: String) -> io::Result<()> {
        match self.state {
            State::Inside(ref room) => {
                // write chat to redis
                // how to distribute to others in the room?
                // plan:
                // message+room goes into a channel, stream is in arc
                // each app will have a write handler with the Arc<stream> in a seperate broker task
                // this broker will receive events
                // each time a peer connects, create a new channel, pass the receiver to a write loop
                // sender stored in hashmap, owned for all connections
                // On each message event, send message using peers sender, this ends up in write loop
                // mentioned above
            }
            State::Outside => not_in_room(stream).await?,
        }
        Ok(())
    }

    async fn handle_join(&mut self, stream: &mut TcpStream, room: String) -> io::Result<()> {
        match self.state {
            State::Inside(ref current_room) => {
                // Notify current room of leaving
                // Notify new room of joining
                // Update state

                self.state = State::Inside(room)
            }
            State::Outside => {
                // Notify new room of joining
                // Update state

                self.state = State::Inside(room)
            }
        }

        Ok(())
    }

    async fn handle_leave(&mut self, stream: &mut TcpStream) -> io::Result<()> {
        match self.state {
            State::Inside(ref room) => {
                // Notify room of leaving
                // Update state

                self.state = State::Outside
            }
            State::Outside => not_in_room(stream).await?,
        }

        Ok(())
    }
}

async fn greeting(stream: &mut TcpStream) -> io::Result<()> {
    let greeting = b"Welcome to ChatsApp!
Enter \">help\" for a list of commands and their usage.\n\n\n";

    stream.write_all(greeting).await?;

    Ok(())
}

async fn invalid(stream: &mut TcpStream) -> io::Result<()> {
    let invalid = b"Invalid command.
Enter \">help\" for a list of commands and their usage.\n";

    stream.write_all(invalid).await?;

    Ok(())
}

async fn help(stream: &mut TcpStream) -> io::Result<()> {
    let help = b"\
Commands:
>help              - Display commands
>exit              - Close connection
>list              - List rooms
>me                - Your user info
>set-username name - Set username
>create-room room  - Create room
>join-room room    - Join room\n";

    stream.write_all(help).await?;

    Ok(())
}

async fn rooms(stream: &mut TcpStream, list: HashSet<String>) -> io::Result<()> {
    let mut res = String::new();

    for room in list {
        res.push_str(&room);
        res.push_str("\n");
    }

    stream.write_all(res.as_bytes()).await?;

    Ok(())
}

async fn error(stream: &mut TcpStream, error: impl std::error::Error) -> io::Result<()> {
    stream.write_all(error.to_string().as_bytes()).await?;

    Ok(())
}

async fn not_in_room(stream: &mut TcpStream) -> io::Result<()> {
    stream
        .write_all(b"You're not currently in a room.\n")
        .await?;

    Ok(())
}
