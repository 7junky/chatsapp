use chatsapp::app::App;
use tokio::{io, net::TcpListener};

#[tokio::main]
async fn main() -> io::Result<()> {
    let listener = TcpListener::bind("0.0.0.0:8000").await?;

    loop {
        let (stream, addr) = listener.accept().await?;
        tokio::spawn(async move {
            let mut app = App::new(addr);

            if let Err(e) = app.run(stream).await {
                eprintln!("{}", e)
            };
        });
    }
}
