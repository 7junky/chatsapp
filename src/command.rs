#[derive(Debug, PartialEq)]
pub enum Command {
    Help,
    List,
    Me,
    SetUsername(String),
    CreateRoom(String),
    JoinRoom(String),
    Message(String),
    Leave,
    Invalid,
    Exit,
}

const HELP: &str = ">help";
const EXIT: &str = ">exit";
const LIST: &str = ">list";
const ME: &str = ">me";
const LEAVE: &str = ">leave";
const SET_USERNAME: &str = ">set-username";
const CREATE_ROOM: &str = ">create-room";
const JOIN_ROOM: &str = ">join-room";

impl Command {
    ///
    ///
    /// # Examples
    ///
    /// ```
    /// use chatsapp::command::Command;
    ///
    /// let c1 = Command::parse(">help".into());
    /// let c2 = Command::parse(">set-username bob".into());
    /// let c3 = Command::parse(">not a command".into());
    ///
    /// assert_eq!(c1, Command::Help);
    /// assert_eq!(c2, Command::SetUsername("bob".to_owned()));
    /// assert_eq!(c3, Command::Invalid);
    /// ```
    pub fn parse(s: String) -> Self {
        if !s.starts_with(">") {
            return Command::Message(s);
        }

        // These commands don't require extra args
        match s.as_str() {
            HELP => return Command::Help,
            EXIT => return Command::Exit,
            LIST => return Command::List,
            LEAVE => return Command::Leave,
            ME => return Command::Me,
            _ => {}
        };

        let (command, rest) = match s.split_once(" ") {
            Some(s) => s,
            None => return Command::Invalid,
        };

        match command {
            // TODO: make sure username is valid
            SET_USERNAME => Command::SetUsername(rest.into()),
            CREATE_ROOM => Command::CreateRoom(rest.into()),
            JOIN_ROOM => Command::JoinRoom(rest.into()),
            _ => Command::Invalid,
        }
    }
}
