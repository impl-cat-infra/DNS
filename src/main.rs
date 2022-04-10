mod parser;

use std::net::SocketAddr;

use log::debug;
use log::info;
use structopt::StructOpt;
use tokio::net::UdpSocket;

#[derive(StructOpt)]
struct Args {
    #[structopt(short, long, default_value="53")]
    port: u16,

    #[structopt(short, long, default_value="0.0.0.0")]
    host: String,
}

async fn handle(buf: Vec<u8>, remote: SocketAddr) -> anyhow::Result<()> {
    debug!("Recieved from {}", remote);
    debug!("{:?}", buf);
    let parsed = match parser::parse(buf.as_slice()) {
        Ok((_, parsed)) => parsed,
        Err(e) => {
            log::error!("Malformed request: {}", e);
            return Ok(())
        }
    };

    log::debug!("Request: {:?}", parsed);
    Ok(())
}

#[paw::main]
#[tokio::main]
async fn main(args: Args) -> anyhow::Result<()> {
    env_logger::init();
    info!("Listening on {}:{}...", args.host, args.port);
    let socket = UdpSocket::bind((args.host, args.port)).await?;
    info!("Socket open");

    loop {
        let mut buf = vec![0; 65536];
        let (len, remote) = socket.recv_from(&mut buf).await?;
        buf.resize(len, 0);

        tokio::spawn(handle(buf, remote));
    }
}