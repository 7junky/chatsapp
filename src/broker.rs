use std::{collections::HashMap, sync::Arc};

use tokio::{
    io::{self, AsyncWrite},
    sync::mpsc::{Receiver, Sender},
};

use crate::app::User;

pub enum Event<T> {
    NewPeer { user: String, stream: Arc<T> },
}

// Shouldn't this be per room?
async fn broker<T>(mut events: Receiver<Event<T>>) -> io::Result<()>
where
    T: AsyncWrite,
{
    // <User, Sender from the User>
    let mut peers: HashMap<String, Sender<String>> = HashMap::new();

    while let Some(event) = events.recv().await {
        //
    }
    todo!()
}
