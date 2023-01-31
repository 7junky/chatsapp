use std::sync::Arc;

use chatsapp::{app::App, broker};
use redis::Client as RedisClient;
use tokio::{io, net::TcpListener};

#[tokio::main]
async fn main() -> io::Result<()> {
    let listener = TcpListener::bind("0.0.0.0:8000").await?;

    let redis = RedisClient::open("redis://:redis@127.0.0.1/").unwrap();
    let redis = Arc::new(redis);

    let rooms = match broker::bootstrap_rooms(&redis).await {
        Ok(r) => r,
        Err(e) => panic!("{}", e),
    };

    loop {
        let redis = Arc::clone(&redis);
        let rooms = Arc::clone(&rooms);

        let (stream, addr) = listener.accept().await?;
        tokio::spawn(async move {
            let mut app = App::new(addr, redis);

            if let Err(e) = app.run(stream, rooms).await {
                eprintln!("{}", e)
            };
        });
    }
}
