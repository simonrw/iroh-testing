use std::{net::IpAddr, str::FromStr};

use clap::Parser;
use color_eyre::eyre::{self, Context, ContextCompat};
use iroh::{
    Endpoint, RelayMap, RelayUrl,
    address_lookup::MemoryLookup,
    endpoint::{Connection, presets},
    protocol::{AcceptError, ProtocolHandler, Router},
};
use iroh_relay::server::{RelayConfig, Server, ServerConfig};
use iroh_tickets::endpoint::EndpointTicket;
use noq::StreamAffinity;

const ALPN: &[u8] = b"iroh-testing/0.1.0\0";

#[derive(Parser)]
struct ClientArgs {
    #[arg(short, long)]
    ticket: String,

    #[arg(long, default_value = "http")]
    scheme: String,

    /// Address to bind the relay HTTP server to
    #[arg(long, default_value = "127.0.0.1")]
    host: String,

    /// Port to bind the relay HTTP server to
    #[arg(long, default_value_t = 8383)]
    port: u16,
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
struct ServerArgs {
    #[arg(long, default_value = "http")]
    scheme: String,

    /// Address to bind the relay HTTP server to
    #[arg(long, default_value = "127.0.0.1")]
    host: String,

    /// Port to bind the relay HTTP server to
    #[arg(long, default_value_t = 8383)]
    port: u16,
}

#[derive(Parser)]
enum Args {
    Client(ClientArgs),
    Server(ServerArgs),
    Relay(RelayArgs),
}

#[tokio::main]
async fn main() -> eyre::Result<()> {
    let args = Args::parse();

    tracing_subscriber::fmt::init();
    color_eyre::install()?;

    match args {
        Args::Client(client_args) => run_client(&client_args).await?,
        Args::Server(server_args) => run_server(&server_args).await?,
        Args::Relay(relay_args) => run_relay(&relay_args).await?,
    }

    Ok(())
}

async fn create_endpoint(scheme: &str, host: &str, port: u16) -> eyre::Result<Endpoint> {
    let host_str = format!("{scheme}://{host}:{port}");
    let relay_url = RelayUrl::from_str(&host_str).unwrap();
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
    let ep = create_endpoint(&args.scheme, &args.host, args.port)
        .await
        .wrap_err("creating endpoint")?;
    ep.online().await;

    let ticket: EndpointTicket = args.ticket.parse().wrap_err("invalid ticket")?;

    tracing::debug!("connecting");
    let conn = ep.connect(ticket, ALPN).await.wrap_err("connecting")?;
    tracing::debug!("connected");

    tracing::debug!("finding direct paths");
    let mut paths = conn.paths_stream();
    let direct_path = paths
        .wait_any(|p| p.is_ip())
        .await
        .wrap_err("connection closed")?;

    tracing::debug!("using direct path: {}", direct_path.id());
    eprintln!("remote address: {}", direct_path.remote_addr());
    eprintln!("local address: {:?}", direct_path.local_addr());
    eprintln!("is selected: {}", direct_path.is_selected());
    eprintln!("is_ip: {}", direct_path.is_ip());
    eprintln!("is_relay: {}", direct_path.is_relay());
    eprintln!("rtt: {:?}", direct_path.rtt());

    let (mut send_stream, mut recv_stream) = conn.open_bi().await.wrap_err("open bi")?;
    send_stream.set_affinity(StreamAffinity::Sticky(direct_path.id()))?;

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

async fn run_server(args: &ServerArgs) -> eyre::Result<()> {
    tracing::debug!("running server");
    let ep = create_endpoint(&args.scheme, &args.host, args.port)
        .await
        .wrap_err("creating endpoint")?;
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
