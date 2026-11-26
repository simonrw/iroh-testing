use std::{
    net::{IpAddr, Ipv4Addr},
    str::FromStr,
};

use clap::Parser;
use color_eyre::eyre::{self, Context};
use iroh::{
    Endpoint, EndpointAddr, EndpointId, RelayMap, RelayUrl,
    address_lookup::{DnsAddressLookup, PkarrPublisher},
    dns::{DnsResolver, NameserverConfig},
    endpoint::{Builder, presets},
    protocol::Router,
};
use iroh_relay::server::{RelayConfig, Server, ServerConfig};
use iroh_tickets::endpoint::EndpointTicket;

mod protocols;

#[derive(Parser)]
struct ClientArgs {
    #[arg(short, long)]
    ticket: String,

    #[arg(short, long)]
    protocol: protocols::Protocols,

    #[arg(long, default_value = "http")]
    scheme: String,

    /// Address to bind the relay HTTP server to
    #[arg(long, default_value = "127.0.0.1")]
    host: String,

    /// Port to bind the relay HTTP server to
    #[arg(long, default_value_t = 8383)]
    port: u16,

    /// Wait for relay connection before sending
    #[arg(short, long)]
    wait: bool,
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
struct DnsArgs {}

#[derive(Parser)]
enum Args {
    Client(ClientArgs),
    Server(ServerArgs),
    Relay(RelayArgs),
    Dns(DnsArgs),
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
        Args::Dns(dns_args) => run_dns(&dns_args).await?,
    }

    Ok(())
}

async fn run_dns(_args: &DnsArgs) -> eyre::Result<()> {
    let mut config = iroh_dns_server::config::Config::default();
    config.dns.port = 5900;
    config.dns.origins = vec!["foo.".to_string(), ".".to_string()];
    config.dns.default_soa =
        "dns1.irohdns.example hostmaster.irohdns.example 0 10800 3600 604800 3600".to_string();
    let server = iroh_dns_server::Server::bind(config)
        .await
        .wrap_err("binding server")?;
    server.join().await?;
    Ok(())
}

/// Builds an endpoint that uses the local relay and resolves endpoint ids through the local DNS server.
fn endpoint_builder(scheme: &str, host: &str, port: u16) -> Builder {
    let host_str = format!("{scheme}://{host}:{port}");
    let relay_url = RelayUrl::from_str(&host_str).unwrap();
    let relay_map = RelayMap::from_iter(vec![relay_url]);

    let addr: Ipv4Addr = "127.0.0.1".parse().unwrap();
    let nameserver = NameserverConfig::udp(IpAddr::V4(addr)).with_port(5900);

    let resolver = DnsResolver::builder()
        .add_nameserver_config(nameserver)
        .build();
    let address_lookup = DnsAddressLookup::builder("foo.".to_string())
        .dns_resolver(resolver)
        .build();

    Endpoint::builder(presets::Minimal)
        .address_lookup(address_lookup)
        .relay_mode(iroh::RelayMode::Custom(relay_map))
    // .relay_mode(iroh::RelayMode::Disabled)
}

enum EndpointToken {
    Ticket(EndpointTicket),
    Id(EndpointId),
}

impl From<EndpointToken> for EndpointAddr {
    fn from(value: EndpointToken) -> Self {
        match value {
            EndpointToken::Ticket(endpoint_ticket) => endpoint_ticket.into(),
            EndpointToken::Id(public_key) => public_key.into(),
        }
    }
}

fn parse_token(input: &str) -> eyre::Result<EndpointToken> {
    if let Ok(ticket) = input.parse::<EndpointTicket>() {
        return Ok(EndpointToken::Ticket(ticket));
    }

    if let Ok(id) = input.parse::<EndpointId>() {
        return Ok(EndpointToken::Id(id));
    }

    eyre::bail!("Bad input for ticket or id: {input}");
}

async fn run_client(args: &ClientArgs) -> eyre::Result<()> {
    tracing::debug!("running client");
    let ep = endpoint_builder(&args.scheme, &args.host, args.port)
        .bind()
        .await
        .wrap_err("creating endpoint")?;
    if args.wait {
        ep.online().await;
    }

    let token = parse_token(&args.ticket).wrap_err("invalid token")?;
    // let ticket: EndpointTicket = args.ticket.parse().wrap_err("invalid ticket")?;

    tracing::debug!("connecting");
    let conn = ep
        .connect(token, args.protocol.alpn())
        .await
        .wrap_err("connecting")?;
    tracing::debug!("connected");

    for path in conn.paths().iter() {
        tracing::debug!("using direct path: {}", path.id());
        eprintln!("remote address: {}", path.remote_addr());
        eprintln!("local address: {:?}", path.local_addr());
        eprintln!("is selected: {}", path.is_selected());
        eprintln!("is_ip: {}", path.is_ip());
        eprintln!("is_relay: {}", path.is_relay());
        eprintln!("rtt: {:?}", path.rtt());
    }

    let (mut send_stream, mut recv_stream) = conn.open_bi().await.wrap_err("open bi")?;
    // send_stream.set_affinity(StreamAffinity::Sticky(direct_path.id()))?;

    tracing::debug!("sending request");
    send_stream
        .write_all(b"hello")
        .await
        .wrap_err("sending hello")?;
    tracing::debug!("reading response");
    send_stream.finish()?;

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

async fn run_server(args: &ServerArgs) -> eyre::Result<()> {
    tracing::debug!("running server");

    // The DNS server only serves records that endpoints have published to its pkarr HTTP API
    let publisher = PkarrPublisher::builder("http://127.0.0.1:8080/pkarr".parse().unwrap());

    let ep = endpoint_builder(&args.scheme, &args.host, args.port)
        .address_lookup(publisher)
        .bind()
        .await
        .wrap_err("creating endpoint")?;
    ep.online().await;

    let addr = ep.addr();
    println!("endpoint id: {}", ep.id());
    dbg!(&addr.relay_urls().collect::<Vec<_>>());
    let ticket = EndpointTicket::new(addr);
    println!("Connect with {ticket}");

    let router = Router::builder(ep)
        .accept(protocols::echo::ALPN, protocols::echo::Echo)
        .accept(protocols::reverse::ALPN, protocols::reverse::Reverse)
        .spawn();

    tokio::signal::ctrl_c().await?;

    router.shutdown().await?;
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
