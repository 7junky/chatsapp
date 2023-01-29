use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::Arc;

use redis::Client as RedisClient;
use tokio::io::{self, AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

use crate::command::Command;
use crate::room;

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

    pub async fn run(&mut self, mut stream: TcpStream) -> io::Result<()> {
        let (mut reader, mut writer) = stream.split();

        greeting(&mut writer).await?;

        let shared_writer = Arc::new(&writer);

        let buf_reader = BufReader::new(&mut reader);
        let mut lines = buf_reader.lines();

        while let Some(message) = lines.next_line().await? {
            let command = Command::parse(message);

            match command {
                Command::Help => {
                    help(&mut writer).await?;
                }
                Command::List => {
                    match room::list(&self.redis).await {
                        Ok(list) => rooms(&mut writer, list).await?,
                        Err(e) => error(&mut writer, e).await?,
                    };
                }
                Command::Me => {
                    self.user_info(&mut writer).await?;
                }
                Command::SetUsername(username) => {
                    self.user.username = Some(username);
                }
                Command::CreateRoom(room) => {
                    if let Err(e) = room::new(&self.redis, &room).await {
                        error(&mut writer, e).await?
                    };
                }
                Command::JoinRoom(room) => {
                    self.handle_join(&mut writer, room).await?;
                }
                Command::Message(msg) => {
                    self.handle_message(&mut writer, msg).await?;
                }
                Command::Leave => {
                    self.handle_leave(&mut writer).await?;
                }
                Command::Invalid => {
                    invalid(&mut writer).await?;
                }
                Command::Exit => break,
            }
        }

        Ok(())
    }

    async fn user_info<T>(&self, stream: &mut T) -> io::Result<()>
    where
        T: AsyncWrite + Unpin,
    {
        let info = format!(
            "Username: {:?}, IP: {}\n",
            self.user.username, self.user.addr
        );

        stream.write_all(info.as_bytes()).await?;

        Ok(())
    }

    async fn handle_message<T>(&mut self, stream: &mut T, msg: String) -> io::Result<()>
    where
        T: AsyncWrite + Unpin,
    {
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

    async fn handle_join<T>(&mut self, stream: &mut T, room: String) -> io::Result<()>
    where
        T: AsyncWrite + Unpin,
    {
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

    async fn handle_leave<T>(&mut self, stream: &mut T) -> io::Result<()>
    where
        T: AsyncWrite + Unpin,
    {
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

async fn greeting<T>(stream: &mut T) -> io::Result<()>
where
    T: AsyncWrite + Unpin,
{
    let greeting = b"Welcome to ChatsApp!
Enter \">help\" for a list of commands and their usage.\n\n\n";

    stream.write_all(greeting).await?;

    Ok(())
}

async fn invalid<T>(stream: &mut T) -> io::Result<()>
where
    T: AsyncWrite + Unpin,
{
    let invalid = b"Invalid command.
Enter \">help\" for a list of commands and their usage.\n";

    stream.write_all(invalid).await?;

    Ok(())
}

async fn help<T>(stream: &mut T) -> io::Result<()>
where
    T: AsyncWrite + Unpin,
{
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

async fn rooms<T>(stream: &mut T, list: HashSet<String>) -> io::Result<()>
where
    T: AsyncWrite + Unpin,
{
    let mut res = String::new();

    for room in list {
        res.push_str(&room);
        res.push_str("\n");
    }

    stream.write_all(res.as_bytes()).await?;

    Ok(())
}

async fn error<T>(stream: &mut T, error: impl std::error::Error) -> io::Result<()>
where
    T: AsyncWrite + Unpin,
{
    stream.write_all(error.to_string().as_bytes()).await?;

    Ok(())
}

async fn not_in_room<T>(stream: &mut T) -> io::Result<()>
where
    T: AsyncWrite + Unpin,
{
    stream
        .write_all(b"You're not currently in a room.\n")
        .await?;

    Ok(())
}
