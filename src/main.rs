use std::{net::IpAddr, str::FromStr};

use clap::Parser;
use color_eyre::eyre::{self, Context};
use iroh::{
    Endpoint, EndpointAddr, EndpointId, RelayMap, RelayUrl,
    address_lookup::MemoryLookup,
    endpoint::{Connection, presets},
    protocol::{AcceptError, ProtocolHandler, Router},
};
use iroh_relay::server::{RelayConfig, Server, ServerConfig};
use iroh_tickets::endpoint::EndpointTicket;

const ALPN: &[u8] = b"iroh-testing/0.1.0\0";

#[derive(Parser)]
struct ClientArgs {
    #[arg(short, long)]
    ticket: String,
}

#[derive(Parser)]
struct RelayArgs {
    /// Address to bind the relay HTTP server to
    #[arg(long, default_value = "127.0.0.1")]
    host: IpAddr,

    /// Port to bind the relay HTTP server to
    #[arg(long, default_value_t = 8383)]
    port: u16,
}

#[derive(Parser)]
enum Args {
    Client(ClientArgs),
    Server,
    Relay(RelayArgs),
}

#[tokio::main]
async fn main() -> eyre::Result<()> {
    let args = Args::parse();

    tracing_subscriber::fmt::init();
    color_eyre::install()?;

    match args {
        Args::Client(client_args) => run_client(&client_args).await?,
        Args::Server => run_server(&args).await?,
        Args::Relay(relay_args) => run_relay(&relay_args).await?,
    }

    Ok(())
}

async fn create_endpoint() -> eyre::Result<Endpoint> {
    let relay_url = RelayUrl::from_str("http://127.0.0.1:8383").unwrap();
    let relay_map = RelayMap::from_iter(vec![relay_url]);

    let address_lookup = MemoryLookup::new();

    let ep = Endpoint::builder(presets::Minimal)
        .alpns(vec![ALPN.to_vec()])
        .address_lookup(address_lookup)
        .relay_mode(iroh::RelayMode::Custom(relay_map))
        // .relay_mode(iroh::RelayMode::Disabled)
        .bind()
        .await?;
    Ok(ep)
}

async fn run_client(args: &ClientArgs) -> eyre::Result<()> {
    tracing::debug!("running client");
    let ep = create_endpoint().await.wrap_err("creating endpoint")?;
    ep.online().await;

    // let ticket: EndpointTicket = args.ticket.parse().wrap_err("invalid ticket")?;
    let ep_id: EndpointId = args.ticket.parse().wrap_err("invalid endpoint id")?;

    let addr = EndpointAddr::new(ep_id).with_relay_url("http://127.0.0.1:8383".parse()?);

    tracing::debug!("connecting");
    let conn = ep.connect(addr, ALPN).await.wrap_err("connecting")?;
    tracing::debug!("connected");

    for path in conn.paths().iter() {
        eprintln!("Path: {}", path.id());
        eprintln!("\tremote address: {}", path.remote_addr());
        eprintln!("\tlocal address: {:?}", path.local_addr());
        eprintln!("\tis selected: {}", path.is_selected());
        eprintln!("\tis_ip: {}", path.is_ip());
        eprintln!("\tis_relay: {}", path.is_relay());
        eprintln!("\trtt: {:?}", path.rtt());
    }

    let (mut send_stream, mut recv_stream) = conn.open_bi().await.wrap_err("open bi")?;

    tracing::debug!("sending request");
    send_stream
        .write_all(b"hello")
        .await
        .wrap_err("sending hello")?;
    tracing::debug!("reading response");

    let mut buf = [0u8; 10];
    let Some(n_bytes) = recv_stream.read(&mut buf).await? else {
        eyre::bail!("could not read number of bytes read");
    };

    tracing::debug!("got response");
    if let Ok(text) = std::str::from_utf8(&buf[..n_bytes]) {
        println!("Got response: '{text}'");
    } else {
        eprintln!("Could not decode response");
    }

    conn.close(1u8.into(), b"done");
    ep.close().await;

    Ok(())
}

#[derive(Debug, Clone)]
struct Echo;

impl ProtocolHandler for Echo {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        let (mut send, mut recv) = connection.accept_bi().await?;
        let bytes_sent = tokio::io::copy(&mut recv, &mut send).await?;

        tracing::debug!(%bytes_sent, "handled connection");

        send.finish()?;
        connection.closed().await;
        Ok(())
    }
}

async fn run_server(_args: &Args) -> eyre::Result<()> {
    tracing::debug!("running server");
    let ep = create_endpoint().await.wrap_err("creating endpoint")?;
    ep.online().await;

    let addr = ep.addr();
    println!("endpoint id: {}", ep.id());
    dbg!(&addr.relay_urls().collect::<Vec<_>>());
    let ticket = EndpointTicket::new(addr);
    println!("Connect with {ticket}");

    let _router = Router::builder(ep).accept(ALPN, Echo).spawn();

    tokio::signal::ctrl_c().await?;
    Ok(())
}

async fn run_relay(args: &RelayArgs) -> eyre::Result<()> {
    let relay_config = RelayConfig::new((args.host, args.port));
    let mut config = ServerConfig::default();
    config.relay = Some(relay_config);
    let mut server = Server::spawn(config).await?;
    server
        .join()
        .await
        .wrap_err("joining server")?
        .wrap_err("with supervision")?;
    Ok(())
}
