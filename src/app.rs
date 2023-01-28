use std::net::SocketAddr;

use tokio::io::{self, AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

use crate::command::Command;

pub struct App {
    addr: String,
    username: Option<String>,
    state: State,
}

impl App {
    pub fn new(addr: SocketAddr) -> Self {
        Self {
            addr: addr.to_string(),
            username: None,
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
                Command::Help => help(&mut stream).await?,
                Command::List => todo!(),
                Command::Me => self.user_info(&mut stream).await?,
                Command::SetUsername(username) => self.username = Some(username),
                Command::CreateRoom(room) => {
                    //
                }
                Command::JoinRoom(room) => {
                    //
                    self.state = State::Inside(room)
                }
                Command::Message(msg) => self.handle_message(&mut stream, msg).await?,
                Command::Invalid => invalid(&mut stream).await?,
                Command::Exit => break,
            }
        }

        Ok(())
    }

    async fn user_info(&self, stream: &mut TcpStream) -> io::Result<()> {
        stream
            .write_all(format!("Username: {:?}, IP: {}\n", self.username, self.addr).as_bytes())
            .await?;

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
            State::Outside => {
                stream
                    .write_all(b"You're not currently in a room.\n")
                    .await?;
            }
        }
        Ok(())
    }
}

// impl State pattern
enum State {
    Inside(String),
    Outside,
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
